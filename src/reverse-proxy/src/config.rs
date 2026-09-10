use anyhow::{Context, ensure};
use base_io_traits::fs_traits::{FileSystemInterface, FileSystemPath, FileSystemType};
use ed25519_dalek::SigningKey;
use network::network::{proxy::TrustedProxies, utils::create_certifified_keys};
use serde::{Deserialize, Serialize};
use std::{
    net::{SocketAddr, SocketAddrV4, SocketAddrV6},
    path::Path,
};
use x509_cert::{
    Certificate,
    der::{Decode, Encode},
};

pub fn default_backend_s2s() -> SocketAddr {
    "127.0.0.1:8315".parse().unwrap()
}

pub const CONFIG_FILE: &str = "config.json";
pub const HASH_FILE: &str = "trusted-proxies.txt";

// Deliberately no Debug implementation: this contains a private key.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub listen_game_v4: SocketAddrV4,
    pub listen_game_v6: SocketAddrV6,
    #[serde(default = "default_backend_s2s")]
    pub backend_s2s: SocketAddr,
    pub backend_server_name: String,
    pub backend_public_key_hash: String,
    pub private_key: String,
    pub certificate: String,
    pub max_connections: usize,
    pub connect_timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
}

impl Config {
    pub fn create(
        listen_game_v4: SocketAddrV4,
        listen_game_v6: SocketAddrV6,
        backend_hash: String,
    ) -> anyhow::Result<Self> {
        parse_hash(&backend_hash)?;
        let (cert, key) = create_certifified_keys();
        Ok(Self {
            listen_game_v4,
            listen_game_v6,
            backend_s2s: default_backend_s2s(),
            backend_server_name: "localhost".into(),
            backend_public_key_hash: backend_hash,
            private_key: hex::encode(key.to_bytes()),
            certificate: hex::encode(cert.to_der()?),
            max_connections: 1024,
            connect_timeout_seconds: 10,
            idle_timeout_seconds: 30,
        })
    }

    pub async fn load(fs: &dyn FileSystemInterface, dir: &Path) -> anyhow::Result<Self> {
        let data = fs
            .read_file_in(
                &dir.join(CONFIG_FILE),
                FileSystemPath::OfType(FileSystemType::ReadWrite),
            )
            .await
            .context("reading proxy config (run reverse-proxy init first)")?;
        serde_json::from_slice(&data).map_err(|_| anyhow::anyhow!("invalid proxy config"))
    }

    pub fn identity(&self) -> anyhow::Result<(Certificate, SigningKey)> {
        let bytes: [u8; 32] = hex::decode(&self.private_key)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .context("invalid proxy private key")?;
        let key = SigningKey::from_bytes(&bytes);
        let cert = Certificate::from_der(
            &hex::decode(&self.certificate).context("invalid proxy certificate encoding")?,
        )?;
        ensure!(
            cert.tbs_certificate
                .subject_public_key_info
                .subject_public_key
                .as_bytes()
                == Some(key.verifying_key().as_bytes().as_slice()),
            "proxy certificate and key do not match"
        );
        Ok((cert, key))
    }

    pub fn hash(&self) -> anyhow::Result<[u8; 32]> {
        Ok(self
            .identity()?
            .0
            .tbs_certificate
            .subject_public_key_info
            .fingerprint_bytes()?)
    }
}

pub fn parse_hash(value: &str) -> anyhow::Result<[u8; 32]> {
    base::hash::decode_hash(value.trim())
        .context("expected a public-key hash of exactly 64 hex digits")
}

pub async fn import_hash(
    fs: &dyn FileSystemInterface,
    dir: &Path,
    hash: [u8; 32],
) -> anyhow::Result<()> {
    let mut contents = match fs
        .read_file_in(
            &dir.join(HASH_FILE),
            FileSystemPath::OfType(FileSystemType::ReadWrite),
        )
        .await
    {
        Ok(data) => String::from_utf8(data)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err.into()),
    };
    let list = TrustedProxies::from_hash_list(&contents)?;
    if list.public_key_hashes.contains(&hash) {
        return Ok(());
    }
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(&base::hash::fmt_hash(&hash));
    contents.push('\n');
    fs.create_dir(dir).await?;
    fs.write_file(&dir.join(HASH_FILE), contents.into_bytes())
        .await?;
    Ok(())
}

pub struct InitOptions {
    pub listen_game_v4: SocketAddrV4,
    pub listen_game_v6: SocketAddrV6,
    pub backend_hash: String,
    pub backend_s2s: SocketAddr,
}

pub async fn init(
    fs: &dyn FileSystemInterface,
    dir: &Path,
    options: InitOptions,
    keep: bool,
) -> anyhow::Result<()> {
    let config = if keep {
        match fs
            .read_file_in(
                &dir.join(CONFIG_FILE),
                FileSystemPath::OfType(FileSystemType::ReadWrite),
            )
            .await
        {
            Ok(data) => serde_json::from_slice::<Config>(&data)
                .ok()
                .filter(|config| config.identity().is_ok()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        }
    } else {
        None
    };
    let config = match config {
        Some(config) => config,
        None => {
            let mut config = Config::create(
                options.listen_game_v4,
                options.listen_game_v6,
                options.backend_hash,
            )?;
            config.backend_s2s = options.backend_s2s;
            fs.create_dir(dir).await?;
            fs.write_file(&dir.join(CONFIG_FILE), serde_json::to_vec_pretty(&config)?)
                .await?;
            config
        }
    };
    let hash = config.hash()?;
    if keep {
        import_hash(fs, dir, hash).await
    } else {
        fs.write_file(
            &dir.join(HASH_FILE),
            format!("{}\n", base::hash::fmt_hash(&hash)).into_bytes(),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base_fs::filesys::FileSystem;

    #[test]
    fn preserves_key_and_hashes() {
        std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
        let rt = base_io::io::create_runtime();
        let fs = FileSystem::new(
            &rt,
            "org",
            "",
            "DDNet-Rs-Alpha-tests",
            "DDNet-Accounts-tests",
        )
        .unwrap();
        let temp = tempfile::tempdir_in(fs.get_save_path()).unwrap();
        let dir = Path::new(temp.path().file_name().unwrap());
        rt.block_on(async {
            import_hash(&fs, dir, [7; 32]).await.unwrap();
            init(
                &fs,
                dir,
                InitOptions {
                    listen_game_v4: "127.0.0.1:0".parse().unwrap(),
                    listen_game_v6: "[::1]:0".parse().unwrap(),
                    backend_hash: "00".repeat(32),
                    backend_s2s: default_backend_s2s(),
                },
                true,
            )
            .await
            .unwrap();
            let original = fs.read_file(&dir.join(CONFIG_FILE)).await.unwrap();
            init(
                &fs,
                dir,
                InitOptions {
                    listen_game_v4: "127.0.0.1:9999".parse().unwrap(),
                    listen_game_v6: "[::1]:0".parse().unwrap(),
                    backend_hash: "00".repeat(32),
                    backend_s2s: default_backend_s2s(),
                },
                true,
            )
            .await
            .unwrap();
            assert_eq!(
                fs.read_file(&dir.join(CONFIG_FILE)).await.unwrap(),
                original
            );
            import_hash(&fs, dir, [7; 32]).await.unwrap();
            let data = fs.read_file(&dir.join(HASH_FILE)).await.unwrap();
            let hashes =
                TrustedProxies::from_hash_list(std::str::from_utf8(&data).unwrap()).unwrap();
            assert_eq!(hashes.public_key_hashes.len(), 2);
            assert!(
                hashes
                    .public_key_hashes
                    .contains(&Config::load(&fs, dir).await.unwrap().hash().unwrap())
            );
        });
    }

    #[test]
    fn invalid_config_is_recreated_and_invalid_hash_list_is_preserved() {
        std::env::set_current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).unwrap();
        let rt = base_io::io::create_runtime();
        let fs = FileSystem::new(
            &rt,
            "org",
            "",
            "DDNet-Rs-Alpha-tests",
            "DDNet-Accounts-tests",
        )
        .unwrap();
        let temp = tempfile::tempdir_in(fs.get_save_path()).unwrap();
        let dir = Path::new(temp.path().file_name().unwrap());
        rt.block_on(async {
            fs.write_file(&dir.join(HASH_FILE), b"invalid".to_vec())
                .await
                .unwrap();
            assert!(import_hash(&fs, dir, [0; 32]).await.is_err());
            assert_eq!(
                fs.read_file(&dir.join(HASH_FILE)).await.unwrap(),
                b"invalid"
            );
            fs.write_file(&dir.join(CONFIG_FILE), b"invalid".to_vec())
                .await
                .unwrap();
            fs.write_file(&dir.join(HASH_FILE), Vec::new())
                .await
                .unwrap();
            init(
                &fs,
                dir,
                InitOptions {
                    listen_game_v4: "127.0.0.1:0".parse().unwrap(),
                    listen_game_v6: "[::1]:0".parse().unwrap(),
                    backend_hash: "00".repeat(32),
                    backend_s2s: default_backend_s2s(),
                },
                true,
            )
            .await
            .unwrap();
            Config::load(&fs, dir).await.unwrap().identity().unwrap();
            let original_hash = Config::load(&fs, dir).await.unwrap().hash().unwrap();
            fs.write_file(&dir.join(HASH_FILE), b"broken".to_vec())
                .await
                .unwrap();
            init(
                &fs,
                dir,
                InitOptions {
                    listen_game_v4: "127.0.0.1:9999".parse().unwrap(),
                    listen_game_v6: "[::1]:9998".parse().unwrap(),
                    backend_hash: "11".repeat(32),
                    backend_s2s: default_backend_s2s(),
                },
                false,
            )
            .await
            .unwrap();
            let config = Config::load(&fs, dir).await.unwrap();
            assert_ne!(config.hash().unwrap(), original_hash);
            assert_eq!(config.listen_game_v4.port(), 9999);
            let contents = fs.read_file(&dir.join(HASH_FILE)).await.unwrap();
            let hashes =
                TrustedProxies::from_hash_list(std::str::from_utf8(&contents).unwrap()).unwrap();
            assert_eq!(hashes.public_key_hashes.len(), 1);
            assert!(hashes.public_key_hashes.contains(&config.hash().unwrap()));
        });
    }
}
