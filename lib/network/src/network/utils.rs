use ed25519_dalek::{
    SigningKey,
    pkcs8::{DecodePrivateKey, EncodePrivateKey},
};
use rcgen::{CertificateParams, KeyPair, PKCS_ED25519};
use spki::der::{Decode, pem::LineEnding};

fn certificate_for_key(private_key: &SigningKey) -> anyhow::Result<x509_cert::Certificate> {
    let key = private_key.to_pkcs8_pem(LineEnding::LF)?;
    let key_pair = KeyPair::from_pkcs8_pem_and_sign_algo(&key, &PKCS_ED25519)?;
    let cert = CertificateParams::new(vec!["localhost".into()])?.self_signed(&key_pair)?;
    Ok(x509_cert::Certificate::from_der(cert.der())?)
}

pub fn create_certifified_keys() -> (x509_cert::Certificate, SigningKey) {
    let private_key = SigningKey::generate(&mut rand::rngs::OsRng);
    (certificate_for_key(&private_key).unwrap(), private_key)
}

/// Restore an Ed25519 key and rebuild its certificate without accessing storage.
pub fn certified_keys_from_pem(pem: &str) -> anyhow::Result<(x509_cert::Certificate, SigningKey)> {
    let private_key = SigningKey::from_pkcs8_pem(pem)
        .map_err(|_| anyhow::anyhow!("invalid Ed25519 PKCS#8 server private key"))?;
    Ok((certificate_for_key(&private_key)?, private_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_roundtrip_preserves_identity() {
        let (cert, key) = create_certifified_keys();
        let pem = key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let (restored_cert, restored_key) = certified_keys_from_pem(&pem).unwrap();
        assert_eq!(key.to_bytes(), restored_key.to_bytes());
        assert_eq!(
            cert.tbs_certificate
                .subject_public_key_info
                .fingerprint_bytes()
                .unwrap(),
            restored_cert
                .tbs_certificate
                .subject_public_key_info
                .fingerprint_bytes()
                .unwrap()
        );
    }

    #[test]
    fn invalid_pem_is_rejected() {
        assert!(certified_keys_from_pem("invalid key").is_err());
    }
}
