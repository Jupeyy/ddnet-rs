# Reverse proxy

QUIC relay preserving client IPs and account certificates.

```sh
cargo build --release -p reverse-proxy
# Binary: target/release/reverse-proxy
reverse-proxy init --listen-game-v4 0.0.0.0:8310 --listen-game-v6 '[::]:8311' --backend-s2s 127.0.0.1:8315 --backend-hash <server-hash>
reverse-proxy run
reverse-proxy export
reverse-proxy import <proxy-hash>
```

- Files: `proxy/config.json` and `proxy/trusted-proxies.txt` within the game's automatic config storage. `--config-dir` changes this relative directory.
- Server: set `sv.private_key_file` to persist its key; use its startup hash for `--backend-hash`.
- Trust: set `sv.trusted_proxy_hashes_file` to `proxy/trusted-proxies.txt`; restart the server after imports.
- `init` replaces the config/key and trust list; `init --keep` preserves existing config and adds its hash. `import` adds hashes without duplicates.
- S2S: `--backend-s2s` points to the server’s HTTPS control listener (`sv.s2s_port_v4/v6`, defaults 8315/8316). It starts only with trusted proxies.
- The proxy registers fresh server info with its own addresses/hash every 10 seconds. Set `sv.register false` to hide the backend’s direct listing; stale listings expire when updates stop.
- Clients trust the proxy hash from `export`.

Backend game ports are discovered over S2S at startup. Restart the proxy if those ports change.
Set the backend’s `sv.resource_server_url` to an external assets server (e.g. `assets-download-server`). Clients download directly using the URL in browser info; the proxy forwards no HTTP traffic.
