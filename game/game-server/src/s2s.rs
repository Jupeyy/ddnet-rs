//! HTTPS control endpoint for trusted proxies. Routes are supplied by the caller.
use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};
use network::network::proxy::TrustedProxies;
use rustls::{
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use tokio::{net::TcpListener, task::JoinSet};
use x509_cert::{
    Certificate,
    der::{Decode, Encode},
};

#[derive(Debug)]
struct ProxyVerifier {
    trusted: TrustedProxies,
    provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for ProxyVerifier {
    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let invalid = || rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding);
        let cert = Certificate::from_der(cert).map_err(|_| invalid())?;
        let hash = cert
            .tbs_certificate
            .subject_public_key_info
            .fingerprint_bytes()
            .map_err(|_| invalid())?;
        if !self.trusted.public_key_hashes.contains(&hash) {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::UnknownIssuer,
            ));
        }
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn tls_config(
    cert: &Certificate,
    key: &SigningKey,
    trusted: TrustedProxies,
) -> anyhow::Result<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(ProxyVerifier { trusted, provider }))
        .with_single_cert(
            vec![CertificateDer::from(cert.to_der()?)],
            PrivatePkcs8KeyDer::from(key.to_pkcs8_der()?.as_bytes().to_vec()).into(),
        )?;
    // Every connection must prove possession of a currently trusted key.
    config.send_tls13_tickets = 0;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

#[derive(Clone, Copy)]
pub struct Ports {
    pub game_v4: u16,
    pub game_v6: u16,
}

pub fn router(
    info: Arc<arc_swap::ArcSwap<game_base::server_browser::ServerBrowserInfo>>,
    ports: Ports,
) -> Router {
    Router::new().route(
        "/server-info",
        axum::routing::get(move || {
            let info = info.clone();
            async move {
                axum::Json(game_base::s2s::ServerInfo {
                    browser_info: info.load().as_ref().clone(),
                    game_port_v4: ports.game_v4,
                    game_port_v6: ports.game_v6,
                })
            }
        }),
    )
}

/// Owns the HTTPS runtime and stops all listeners/connections on drop.
pub struct Server {
    rt: Option<tokio::runtime::Runtime>,
    task: tokio::task::JoinHandle<()>,
    addresses: Vec<SocketAddr>,
}

impl Server {
    pub fn new(
        addresses: &[SocketAddr],
        cert: &Certificate,
        key: &SigningKey,
        trusted: TrustedProxies,
        routes: Router,
    ) -> anyhow::Result<Option<Self>> {
        if trusted.public_key_hashes.is_empty() {
            return Ok(None);
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .build()?;
        let bound = rt
            .block_on(BoundServer::bind(addresses, cert, key, trusted))?
            .unwrap();
        let addresses = bound.local_addresses()?;
        let task = rt.spawn(async move {
            if let Err(err) = bound.run(routes).await {
                log::error!("proxy HTTPS server exited: {err}");
            }
        });
        Ok(Some(Self {
            rt: Some(rt),
            task,
            addresses,
        }))
    }

    pub fn local_addresses(&self) -> &[SocketAddr] {
        &self.addresses
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(rt) = self.rt.take() {
            self.task.abort();
            let _ = rt.block_on(&mut self.task);
            rt.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

struct BoundServer {
    listeners: Vec<TcpListener>,
    tls: RustlsConfig,
}

impl BoundServer {
    /// Bind only when proxies are trusted. Bind errors fail game-server startup.
    pub async fn bind(
        addresses: &[SocketAddr],
        cert: &Certificate,
        key: &SigningKey,
        trusted: TrustedProxies,
    ) -> anyhow::Result<Option<Self>> {
        if trusted.public_key_hashes.is_empty() {
            return Ok(None);
        }
        let tls = RustlsConfig::from_config(Arc::new(tls_config(cert, key, trusted)?));
        let mut listeners = Vec::new();
        for addr in addresses {
            listeners.push(TcpListener::bind(addr).await?);
            log::info!("proxy HTTPS listening on {addr}");
        }
        Ok(Some(Self { listeners, tls }))
    }

    pub fn local_addresses(&self) -> std::io::Result<Vec<SocketAddr>> {
        self.listeners.iter().map(TcpListener::local_addr).collect()
    }

    /// Dropping this future closes listeners and all connection tasks.
    pub async fn run(self, routes: Router) -> anyhow::Result<()> {
        let mut listeners = JoinSet::new();
        for listener in self.listeners {
            let mut server = axum_server::from_tcp_rustls(listener.into_std()?, self.tls.clone())?;
            server
                .http_builder()
                .http1()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(Duration::from_secs(10));
            listeners.spawn(server.serve(routes.clone().into_make_service()));
        }
        if let Some(result) = listeners.join_next().await {
            result??;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use network::network::utils::create_certifified_keys;
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    #[tokio::test]
    async fn empty_trust_does_not_bind() {
        let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (cert, key) = create_certifified_keys();
        assert!(
            BoundServer::bind(
                &[occupied.local_addr().unwrap()],
                &cert,
                &key,
                TrustedProxies::default()
            )
            .await
            .unwrap()
            .is_none()
        );
    }

    #[tokio::test]
    async fn requires_trusted_client_key() {
        let (server_cert, server_key) = create_certifified_keys();
        let (proxy_cert, proxy_key) = create_certifified_keys();
        let hash = proxy_cert
            .tbs_certificate
            .subject_public_key_info
            .fingerprint_bytes()
            .unwrap();
        for (send_cert, trusted) in [(true, true), (true, false), (false, true)] {
            let expected = send_cert && trusted;
            let trusted = TrustedProxies {
                public_key_hashes: [if trusted { hash } else { [0; 32] }].into(),
            };
            let server = TlsAcceptor::from(Arc::new(
                tls_config(&server_cert, &server_key, trusted).unwrap(),
            ));
            let mut roots = rustls::RootCertStore::empty();
            roots
                .add(CertificateDer::from(server_cert.to_der().unwrap()))
                .unwrap();
            let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_root_certificates(roots);
            let client = if send_cert {
                builder
                    .with_client_auth_cert(
                        vec![CertificateDer::from(proxy_cert.to_der().unwrap())],
                        PrivatePkcs8KeyDer::from(
                            proxy_key.to_pkcs8_der().unwrap().as_bytes().to_vec(),
                        )
                        .into(),
                    )
                    .unwrap()
            } else {
                builder.with_no_client_auth()
            };
            let client = TlsConnector::from(Arc::new(client));
            let (a, b) = tokio::io::duplex(65536);
            let (server_result, _) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(
                    server.accept(a),
                    client.connect("localhost".try_into().unwrap(), b)
                )
            })
            .await
            .unwrap();
            assert_eq!(server_result.is_ok(), expected);
        }
    }
}
