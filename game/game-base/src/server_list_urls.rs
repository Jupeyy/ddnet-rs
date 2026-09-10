use anyhow::Context;
use base_io_traits::fs_traits::{FileSystemInterface, FileSystemPath, FileSystemType};
use url::Url;

pub fn defaults() -> Vec<Url> {
    vec![Url::parse("https://pg.ddnet.org:4444/ddnet/15/").unwrap()]
}

/// One HTTPS base URL per line, relative to writable storage.
pub async fn load(fs: &dyn FileSystemInterface) -> Vec<Url> {
    let contents = match fs
        .read_file_in(
            "server_list_urls.cfg".as_ref(),
            FileSystemPath::OfType(FileSystemType::ReadWrite),
        )
        .await
    {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(contents) => contents,
            Err(err) => {
                log::error!("invalid server_list_urls.cfg encoding: {err}");
                return defaults();
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return defaults();
        }
        Err(err) => {
            log::error!("could not read server_list_urls.cfg: {err}");
            return defaults();
        }
    };
    parse(&contents).unwrap_or_else(|err| {
        log::error!("invalid server_list_urls.cfg: {err:#}");
        defaults()
    })
}

fn parse(contents: &str) -> anyhow::Result<Vec<Url>> {
    let urls: Vec<Url> = contents
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line = line.trim();
            (!line.is_empty() && !line.starts_with('#')).then_some((index, line))
        })
        .map(|(index, line)| {
            let mut url = Url::parse(line)
                .with_context(|| format!("invalid server list URL on line {}", index + 1))?;
            anyhow::ensure!(
                url.scheme() == "https" && url.host_str().is_some(),
                "server list URL on line {} must use HTTPS",
                index + 1
            );
            if !url.path().ends_with('/') {
                url.set_path(&format!("{}/", url.path()));
            }
            Ok(url)
        })
        .collect::<anyhow::Result<_>>()?;
    Ok(if urls.is_empty() { defaults() } else { urls })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls_and_endpoints() {
        let urls = parse(
            "# local masters\nhttps://localhost:4444/ddnet/15\n\nhttps://[::1]:4445/custom/\n",
        )
        .unwrap();
        assert_eq!(urls.len(), 2);
        assert_eq!(
            urls[0].join("servers.json").unwrap().as_str(),
            "https://localhost:4444/ddnet/15/servers.json"
        );
        assert_eq!(
            urls[1].join("register").unwrap().as_str(),
            "https://[::1]:4445/custom/register"
        );
        assert_eq!(parse("").unwrap(), defaults());
        assert_eq!(parse("# no entries\n").unwrap(), defaults());
        assert!(parse("http://localhost:4444/").is_err());
        assert!(parse("invalid").is_err());
    }
}
