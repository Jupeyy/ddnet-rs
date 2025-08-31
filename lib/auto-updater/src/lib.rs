use anyhow::anyhow;
use base::hash::decode_hash;
use base_io::io::Io;
use base_io::runtime::IoRuntimeTask;
use base_io_traits::http_traits::HttpClientInterface;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::env::consts::{ARCH, OS};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;

/// Determine the current binary's stem name.
fn current_bin_name() -> anyhow::Result<String> {
    let exe = std::env::current_exe()?;
    let stem = exe
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("Failed to get current binary name"))?;
    Ok(stem.to_string())
}

/// matches github actions `runner.arch` style
fn normalized_arch() -> &'static str {
    match ARCH {
        "x86" => "x86",
        "x86_64" => "x64",
        "arm" => "arm",
        "aarch64" => "arm64",
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitTag(String);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AutoUpdateInfo {
    git_tag: GitTag,
    last_check: DateTime<Utc>,
}

type DownloadResult = (GitTag, Vec<u8>, DateTime<Utc>);

const INFO_NAME: &str = "auto_updater_info.json";

pub struct AutoUpdater {
    current_update: Arc<Mutex<Option<DownloadResult>>>,
    io: Io,
    // drop last
    _task: IoRuntimeTask<()>,
}

impl AutoUpdater {
    pub fn new(io: &Io, owner: &str, repo: &str, tag: &str, suffix: &str) -> Self {
        let http = io.http.clone();
        let fs = io.fs.clone();
        let owner = owner.to_string();
        let repo = repo.to_string();
        let tag = tag.to_string();
        let suffix = suffix.to_string();
        let current_update: Arc<Mutex<Option<DownloadResult>>> = Default::default();
        let current_update_task = current_update.clone();
        let task = io
            .rt
            .spawn(async move {
                let file = fs.read_file(INFO_NAME.as_ref()).await.ok();
                let mut info: Option<AutoUpdateInfo> =
                    file.and_then(|f| serde_json::from_slice(&f).ok());
                loop {
                    let now = Utc::now();
                    // by default, check every 12 hours
                    let mut update_interval = std::time::Duration::from_secs(60 * 60 * 12);
                    if info
                        .as_ref()
                        .is_none_or(|info| now >= info.last_check + Duration::days(1))
                    {
                        // allowed to update
                        match Self::download_update(http.as_ref(), &owner, &repo, &tag, &suffix)
                            .await
                        {
                            Ok((git_tag, file, time)) => {
                                // wait a full day
                                update_interval = std::time::Duration::from_secs(60 * 60 * 24);
                                *current_update_task.lock().await =
                                    Some((git_tag.clone(), file, time));

                                info = Some(AutoUpdateInfo {
                                    git_tag,
                                    last_check: time,
                                });
                            }
                            Err(err) => {
                                log::error!("Failed to download update: {err}");
                                // on error try again after 2 hours
                                update_interval = std::time::Duration::from_secs(60 * 60 * 2);
                            }
                        }
                    }

                    tokio::time::sleep(update_interval).await;
                }
            })
            .abortable();
        Self {
            _task: task,
            current_update,
            io: io.clone(),
        }
    }

    /// Perform a self-update from the given owner/repo on GitHub.
    /// Optionally target a specific tag.
    /// Returns a human-readable success message.
    async fn download_update(
        http: &dyn HttpClientInterface,
        owner: &str,
        repo: &str,
        tag: &str,
        suffix: &str,
    ) -> anyhow::Result<DownloadResult> {
        let bin_name = current_bin_name()?;

        let tag_url = Url::parse("https://github.com")?
            .join(&format!("{owner}/{repo}/releases/download/{tag}/"))?;

        let checksums = http
            .download_text(tag_url.join("blake3_checksum.txt")?)
            .await?;

        let asset_name = format!("{bin_name}-{OS}-{}{suffix}.zip", normalized_arch());

        let mut lines = checksums.lines();
        let git_tag = lines
            .next()
            .ok_or_else(|| anyhow!("checksum file did not contain git tag as first line"))?;

        let asset_hash = lines
            .find_map(|line| {
                let (name, hash) = line.split_once(|c| char::is_ascii_whitespace(&c))?;
                if name == asset_name {
                    Some(hash)
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("No asset's hash found for: {asset_name}"))?;
        let asset_hash = decode_hash(asset_hash)
            .ok_or_else(|| anyhow!("Asset hash was not a valid hash string"))?;

        // follows:
        // https://github.com/{owner}/{repo}/releases/download/{tag}/{asset_name}
        let zip_archive = http
            .download_binary(tag_url.join(&asset_name)?, &asset_hash)
            .await?;

        // unzip and extract the binary matching `bin_name`
        let mut zip = zip::ZipArchive::new(Cursor::new(zip_archive))
            .map_err(|e| anyhow!("Failed to read zip archive: {e}"))?;

        let mut extracted = None;
        for i in 0..zip.len() {
            let mut file = zip
                .by_index(i)
                .map_err(|e| anyhow!("Zip entry error: {e}"))?;
            if file.is_dir() {
                continue;
            }
            let name_in_zip = file.enclosed_name();

            if name_in_zip.is_some_and(|n| n.as_path() == Path::new(&bin_name)) {
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)?;
                extracted = Some(buf);
                break;
            }
        }

        let file =
            extracted.ok_or_else(|| anyhow!("Binary '{bin_name}' not found in downloaded zip"))?;

        Ok((GitTag(git_tag.into()), file, Utc::now()))
    }

    fn blocking_replace(file: &[u8]) -> anyhow::Result<()> {
        // Create a temporary file to hold the new executable
        let mut tmp = tempfile::NamedTempFile::new()
            .map_err(|e| anyhow!("Failed to create temp file: {e}"))?;

        // Copy decompressed bytes into the temp file
        tmp.write_all(file)
            .map_err(|e| anyhow!("Failed to write decompressed binary: {e}"))?;

        // Ensure execute permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tmp.as_file().metadata()?.permissions();
            // rwxr-xr-x
            perms.set_mode(0o755);
            tmp.as_file().set_permissions(perms)?;
        }

        // Close the handle and keep the path alive until after replace
        let tmp_path = tmp.into_temp_path();

        self_replace::self_replace(&tmp_path)?;

        Ok(())
    }
}

impl Drop for AutoUpdater {
    fn drop(&mut self) {
        let current_update = self.current_update.blocking_lock();
        let Some((git_tag, file, last_check)) = &*current_update else {
            return;
        };
        if let Err(err) = Self::blocking_replace(file) {
            log::error!("Failed to update: {err}");
        } else {
            let fs = self.io.fs.clone();
            let info = AutoUpdateInfo {
                git_tag: git_tag.clone(),
                last_check: *last_check,
            };
            self.io.rt.spawn_without_lifetime(async move {
                fs.write_file(INFO_NAME.as_ref(), serde_json::to_vec_pretty(&info)?)
                    .await?;
                Ok(())
            });
        }
    }
}
