use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::Arc,
};

use anyhow::Context;
use base::hash::{fmt_hash, name_and_hash};
use base_http::{http::HttpClient, http_server::HttpDownloadServer};
use base_io::io::Io;
use base_io_traits::fs_traits::{FileSystemEntryTy, FileSystemInterface};
use clap::Parser;
use game_server::server_game::ServerMap;

#[derive(Parser)]
#[command(about = "Serve game maps, resources and WASM modules from game storage")]
struct Args {
    #[arg(long, default_value_t = 3400)]
    port_v4: u16,
    #[arg(long, default_value_t = 3401)]
    port_v6: u16,
    /// Additional public assets directory, relative to game storage.
    #[arg(long, value_parser = relative_directory)]
    provided_assets: Option<PathBuf>,
}

fn relative_directory(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err("expected a relative assets directory without '..'".into());
    }
    Ok(path)
}

async fn has_directory(fs: &dyn FileSystemInterface, path: &str) -> anyhow::Result<bool> {
    let mut parent = PathBuf::from(".");
    for part in path.split('/') {
        if !matches!(
            fs.entries_in_dir(&parent).await?.get(part),
            Some(FileSystemEntryTy::Directory)
        ) {
            return Ok(false);
        }
        parent.push(part);
    }
    Ok(true)
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let io = Io::new(
        |rt| {
            Arc::new(
                base_fs::filesys::FileSystem::new(
                    rt,
                    "org",
                    "",
                    "DDNet-Rs-Alpha",
                    "DDNet-Accounts",
                )
                .unwrap(),
            )
        },
        Arc::new(HttpClient::new()),
    );
    let fs = io.fs.clone();
    let names = io
        .rt
        .spawn(async move {
            let mut names = BTreeSet::new();
            for (dir, extension) in [("map/maps", ".twmap.tar"), ("legacy/maps", ".map")] {
                if !has_directory(fs.as_ref(), dir).await? {
                    continue;
                }
                for (name, ty) in fs.entries_in_dir(dir.as_ref()).await? {
                    if matches!(ty, FileSystemEntryTy::File { .. })
                        && let Some(name) = name.strip_suffix(extension)
                    {
                        names.insert(name.to_owned());
                    }
                }
            }
            Ok(names)
        })
        .get()?;
    let pool = Arc::new(rayon::ThreadPoolBuilder::new().build()?);
    let mut files = HashMap::new();
    for name in &names {
        let map = ServerMap::new(&name.as_str().try_into()?, &io, &pool)
            .with_context(|| format!("loading map {name}"))?;
        let (name, hash) = name_and_hash(map.name.as_str(), &map.map_file);
        files.insert(
            format!("map/maps/{name}_{}.twmap.tar", fmt_hash(&hash)),
            map.map_file,
        );
        files.extend(map.resource_files);
    }
    let fs = io.fs.clone();
    let extra = io
        .rt
        .spawn(async move {
            let mut files = HashMap::new();
            for dir in ["map/resources", "mods/state", "mods/render", "thumbnails"] {
                if !has_directory(fs.as_ref(), dir).await? {
                    continue;
                }
                for (path, bytes) in fs.files_in_dir_recursive(dir.as_ref()).await? {
                    let path = path.to_str().context("non-UTF8 asset path")?;
                    if dir.starts_with("mods/") && path.ends_with(".wasm") {
                        let (name, hash) =
                            name_and_hash(path.strip_suffix(".wasm").unwrap(), &bytes);
                        files.insert(format!("{dir}/{name}_{}.wasm", fmt_hash(&hash)), bytes);
                    } else {
                        files.insert(format!("{dir}/{path}"), bytes);
                    }
                }
            }
            if let Some(dir) = args.provided_assets {
                for (path, bytes) in fs.files_in_dir_recursive(&dir).await? {
                    files.insert(
                        path.to_str().context("non-UTF8 asset path")?.to_owned(),
                        bytes,
                    );
                }
            }
            Ok(files)
        })
        .get()?;
    files.extend(extra);
    anyhow::ensure!(
        !names.is_empty(),
        "no maps found in map/maps or legacy/maps"
    );
    let server = HttpDownloadServer::new(files, HashMap::new(), args.port_v4, args.port_v6)?;
    println!(
        "serving {} maps and their assets on IPv4 port {} and IPv6 port {}",
        names.len(),
        server.port_v4,
        server.port_v6
    );
    io.rt
        .spawn(async {
            tokio::signal::ctrl_c().await?;
            Ok(())
        })
        .get()?;
    drop(server);
    Ok(())
}
