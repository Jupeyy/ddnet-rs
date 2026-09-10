//! Authenticated forwarding for terminating QUIC proxies.
//!
//! A proxy presents a certificate made with its pinned key, containing the original
//! address and complete client certificate. It must authenticate the client's TLS
//! key possession first. The backend trusts that assertion, while account plugins
//! still verify the original account certificate's signature.
use std::{collections::HashSet, net::SocketAddr};

use anyhow::{bail, ensure};
use ed25519_dalek::{SigningKey, pkcs8::EncodePrivateKey};
use rcgen::{CertificateParams, CustomExtension, KeyPair, PKCS_ED25519};
use spki::der::{Decode, Encode, asn1::OctetString};
use x509_cert::Certificate;

// Project-specific UUID OID. The extension value is a DER OCTET STRING containing
// bincode (version, original address, original certificate DER).
const FORWARDING_OID: &[u64] = &[2, 25, 247823191];
const MAX_FORWARDING_SIZE: usize = 32 * 1024;

/// SHA-256 fingerprints of proxy SubjectPublicKeyInfo, not whole certificates.
#[derive(Debug, Clone, Default)]
pub struct TrustedProxies {
    pub public_key_hashes: HashSet<[u8; 32]>,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardedIdentity {
    pub addr: SocketAddr,
    pub cert: Certificate,
}

/// Create the proxy's client certificate for one forwarded connection.
/// Pass this and `proxy_key` through `NetworkClientCertMode::FromCertAndPrivateKey`.
/// The original certificate is preserved byte-for-byte, including account signatures.
pub fn create_proxy_certificate(
    proxy_key: &SigningKey,
    original_addr: SocketAddr,
    original_cert: &Certificate,
) -> anyhow::Result<Certificate> {
    let payload = bincode::serde::encode_to_vec(
        (1u8, original_addr, original_cert.to_der()?),
        bincode::config::standard(),
    )?;
    ensure!(
        payload.len() <= MAX_FORWARDING_SIZE,
        "forwarded identity too large"
    );
    let key = KeyPair::from_pkcs8_der_and_sign_algo(
        &proxy_key.to_pkcs8_der()?.as_bytes().into(),
        &PKCS_ED25519,
    )?;
    let mut params = CertificateParams::new(vec!["localhost".into()])?;
    params
        .custom_extensions
        .push(CustomExtension::from_oid_content(
            FORWARDING_OID,
            OctetString::new(payload)?.to_der()?,
        ));
    Ok(Certificate::from_der(params.self_signed(&key)?.der())?)
}

impl TrustedProxies {
    /// Parse one SHA-256 SPKI fingerprint (64 hex digits) per line.
    /// Empty lines and lines beginning with `#` are ignored; duplicates are collapsed.
    pub fn from_hash_list(contents: &str) -> anyhow::Result<Self> {
        let mut proxies = Self::default();
        for (index, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let hash = base::hash::decode_hash(line)
                .ok_or_else(|| anyhow::anyhow!("invalid proxy hash on line {}", index + 1))?;
            proxies.public_key_hashes.insert(hash);
        }
        Ok(proxies)
    }

    /// Call only after TLS has authenticated possession of the peer's private key.
    pub(crate) fn resolve(&self, peer: &Certificate) -> anyhow::Result<Option<ForwardedIdentity>> {
        let oid = spki::ObjectIdentifier::new("2.25.247823191")?;
        let mut extensions = peer
            .tbs_certificate
            .extensions
            .iter()
            .flatten()
            .filter(|ext| ext.extn_id == oid);
        let extension = extensions.next();
        let trusted = self.public_key_hashes.contains(
            &peer
                .tbs_certificate
                .subject_public_key_info
                .fingerprint_bytes()?,
        );
        if !trusted {
            ensure!(
                extension.is_none(),
                "untrusted proxy forwarding certificate"
            );
            return Ok(None);
        }
        let Some(extension) = extension else {
            bail!("trusted proxy omitted forwarded identity");
        };
        ensure!(extensions.next().is_none(), "duplicate forwarded identity");
        ensure!(
            extension.extn_value.as_bytes().len() <= MAX_FORWARDING_SIZE + 8,
            "forwarded identity too large"
        );
        let payload = OctetString::from_der(extension.extn_value.as_bytes())?;
        let ((version, addr, cert), consumed): ((u8, SocketAddr, Vec<u8>), _) =
            bincode::serde::decode_from_slice(
                payload.as_bytes(),
                bincode::config::standard().with_limit::<MAX_FORWARDING_SIZE>(),
            )?;
        ensure!(version == 1, "unsupported forwarding version");
        ensure!(
            consumed == payload.as_bytes().len(),
            "trailing forwarding data"
        );
        Ok(Some(ForwardedIdentity {
            addr,
            cert: Certificate::from_der(&cert)?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::utils::create_certifified_keys;

    #[test]
    fn preserves_client_certificate_and_address() {
        let (proxy, key) = create_certifified_keys();
        let (client, _) = create_certifified_keys();
        let trusted = TrustedProxies {
            public_key_hashes: [proxy
                .tbs_certificate
                .subject_public_key_info
                .fingerprint_bytes()
                .unwrap()]
            .into(),
        };
        for addr in ["127.0.0.1:1234", "[2001:db8::1]:5678"] {
            let addr = addr.parse().unwrap();
            let cert = create_proxy_certificate(&key, addr, &client).unwrap();
            let identity = trusted.resolve(&cert).unwrap().unwrap();
            assert_eq!(identity.addr, addr);
            assert_eq!(identity.cert.to_der().unwrap(), client.to_der().unwrap());
            assert!(TrustedProxies::default().resolve(&cert).is_err());
        }
        assert!(trusted.resolve(&proxy).is_err());
        assert!(trusted.resolve(&client).unwrap().is_none());
    }

    #[tokio::test]
    async fn forwards_identity_through_tls_handshake() {
        use crate::network::{
            quinn_network::QuinnEndpointWrapper,
            traits::{
                NetworkConnectionInterface, NetworkEndpointInterface, NetworkIncomingInterface,
            },
            types::{
                NetworkClientCertCheckMode, NetworkClientCertMode, NetworkClientInitOptions,
                NetworkServerCertAndKey, NetworkServerCertMode, NetworkServerInitOptions,
            },
        };
        let (server_cert, server_key) = create_certifified_keys();
        let (proxy_cert, proxy_key) = create_certifified_keys();
        let (client_cert, _) = create_certifified_keys();
        let addr = "[2001:db8::42]:1234".parse().unwrap();
        let trusted = TrustedProxies {
            public_key_hashes: [proxy_cert
                .tbs_certificate
                .subject_public_key_info
                .fingerprint_bytes()
                .unwrap()]
            .into(),
        };
        let (server, _) = QuinnEndpointWrapper::make_server_endpoint(
            "127.0.0.1:0".parse().unwrap(),
            NetworkServerCertMode::FromCertAndPrivateKey(Box::new(NetworkServerCertAndKey {
                cert: server_cert.clone(),
                private_key: server_key,
            })),
            &NetworkServerInitOptions::new().with_trusted_proxies(trusted),
        )
        .unwrap();
        let client = QuinnEndpointWrapper::make_client_endpoint(
            "127.0.0.1:0".parse().unwrap(),
            &NetworkClientInitOptions::new(
                NetworkClientCertCheckMode::CheckByCert {
                    cert: server_cert.to_der().unwrap().into(),
                },
                NetworkClientCertMode::FromCertAndPrivateKey {
                    cert: create_proxy_certificate(&proxy_key, addr, &client_cert).unwrap(),
                    private_key: proxy_key,
                },
            ),
        )
        .unwrap();
        let connecting = client
            .connect(server.sock_addr().unwrap(), "localhost")
            .unwrap();
        let (outgoing, incoming) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::join!(connecting, async {
                server.accept().await.unwrap().accept().unwrap().await
            })
        })
        .await
        .unwrap();
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        assert_eq!(incoming.remote_addr(), addr);
        assert_eq!(
            incoming.transport_remote_addr(),
            client.sock_addr().unwrap()
        );
        assert_eq!(
            incoming.peer_identity().to_der().unwrap(),
            client_cert.to_der().unwrap()
        );
        outgoing
            .close(
                crate::network::errors::ConnectionErrorCode::Shutdown,
                "done",
            )
            .await;
    }

    #[test]
    fn rejects_duplicate_and_malformed_forwarding() {
        let (proxy, key) = create_certifified_keys();
        let trusted = TrustedProxies {
            public_key_hashes: [proxy
                .tbs_certificate
                .subject_public_key_info
                .fingerprint_bytes()
                .unwrap()]
            .into(),
        };
        let mut cert =
            create_proxy_certificate(&key, "127.0.0.1:1234".parse().unwrap(), &proxy).unwrap();
        let extensions = cert.tbs_certificate.extensions.as_mut().unwrap();
        let index = extensions
            .iter()
            .position(|ext| ext.extn_id.to_string() == "2.25.247823191")
            .unwrap();
        extensions.push(extensions[index].clone());
        assert!(trusted.resolve(&cert).is_err());
        let extensions = cert.tbs_certificate.extensions.as_mut().unwrap();
        extensions.pop();
        extensions[index].extn_value = OctetString::new(vec![0xff]).unwrap();
        assert!(trusted.resolve(&cert).is_err());
    }
}
