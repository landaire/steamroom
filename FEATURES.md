# Feature Comparison

Comparison between DepotDownloader (C#) and steamroom (Rust).

## Feature Parity Status

| Feature | DepotDownloader (C#) | steamroom | Notes |
|---|---|---|---|
| Anonymous login | ✅ | ✅ | Auto-detects when no `--username` |
| Authenticated login (password + PKCS1v15 RSA) | ✅ | ✅ | With password retry (3 attempts) |
| SteamGuard 2FA (email/authenticator) | ✅ | ✅ | Prompts for code interactively |
| QR code login | ✅ | ✅ | Terminal QR rendering, polls until scanned |
| Token persistence (remember-password) | ✅ | ✅ | JSON file in `~/.depotdownloader/` |
| CM server discovery (API + fallback) | ✅ | ✅ | Steam Directory API with hardcoded fallback |
| Channel encryption (OAEP-SHA1 + AES-CBC HMAC) | ✅ | ✅ | ECB-encrypted IV, HMAC-SHA1 derivation |
| WebSocket transport | ❌ | ✅ | TLS-encrypted, no custom cipher needed |
| PICS access tokens + product info | ✅ | ✅ | Text KV parsing for app info |
| Depot key retrieval | ✅ | ✅ | From server or local config.vdf |
| CDN server discovery | ✅ | ✅ | Service method RPC |
| CDN auth tokens | ✅ | ✅ | |
| Manifest request codes | ✅ | ✅ | |
| Manifest download + parsing (v5 protobuf) | ✅ | ✅ | |
| Manifest download + parsing (v4 binary) | ✅ | ✅ | Different magic values from v5 |
| Filename decryption (AES-256-ECB/CBC) | ✅ | ✅ | Handles line-wrapped base64 |
| Chunk download (HTTP/2) | ✅ | ✅ | HTTP/2 multiplexing, parallel with semaphore |
| Chunk decryption (AES-256) | ✅ | ✅ | ECB IV + CBC payload |
| Chunk decompression (PKZip) | ✅ | ✅ | |
| Chunk decompression (LZMA/VZip) | ✅ | ✅ | VZstd and VZlzma Valve formats |
| Checksum verification (Adler32 zero-seed) | ✅ | ✅ | Steam's non-standard zero seed |
| File verification (`--verify`) | ✅ | ✅ | SHA-1 + size match skips download |
| File list filtering (`--filelist`) | ✅ | ✅ | Case-insensitive, supports `regex:` prefix |
| Regex file filtering | ✅ | ✅ | `--file-regex` |
| Manifest caching | ✅ | ✅ | `~/.depotdownloader/manifests/` |
| Delta/differential chunk downloads | ✅ | ✅ | Adler-32 per-chunk comparison, fetches only changed chunks |
| Download resumption | ✅ | ✅ | Via chunk-level delta against existing files |
| Depot config persistence | ✅ | ✅ | Tracks installed manifests for delta downloads |
| Workshop download | ✅ | ✅ | Full manifest + chunk pipeline |
| Branch selection | ✅ | ✅ | `--branch` flag |
| Beta/branch passwords | ✅ | ⚠️ | Cached hash from config.vdf via `--local-keys`; plaintext `--branch-password` not yet wired up |
| Encrypted manifest detection | ✅ | ✅ | Shown in `info` output |
| JSON output | ✅ | ✅ | `--format json` on info/manifests/files/diff/packages |
| Lancache detection + proxy | ✅ | ✅ | DNS-based detection |
| Manifest diff | ❌ | ✅ | `diff` command: added/removed/changed files between manifests |
| Package queries | ❌ | ✅ | `packages` command: query sub details by ID |
| Save manifest only | ❌ | ✅ | `save-manifest`: download manifest without content |
| Local manifest reading | ❌ | ✅ | `files --manifest-file` reads saved manifests offline |
| Local Steam credential reuse | ❌ | ✅ | `--use-steam-token`: extracts cached token from local Steam |
| Local depot key reuse | ❌ | ✅ | `--local-keys`: reads depot keys from Steam's config.vdf |
| Local Steam config inspection | ❌ | ✅ | `local-info`: shows cached depot keys and beta branches |
| `--raw-errors` debug mode | ❌ | ✅ | Shows full error chain for troubleshooting |
| Configurable device name | ❌ | ✅ | `--device-name` / `DD_DEVICE_NAME` |
| Legacy CLI compat mode | ❌ | ✅ | `DD_COMPAT=1` flat-argument mode |
| C/C++ FFI bindings | ❌ | ✅ | Via Diplomat, with Python nanobind example |
| Serde deserializer for KV format | ❌ | ✅ | `#[derive(Deserialize)]` from PICS data |
| Proto extraction tool | ❌ | ✅ | Extracts .proto from steamclient64.dll (PE) |
| Fuzz testing | ❌ | ✅ | cargo-fuzz targets for all parsers |
| Account access verification (license check) | ✅ | ❌ | |
| Multi-depot file deduplication | ✅ | ❌ | |
| Custom login ID (`--loginid`) | ✅ | ⚠️ | CLI flag exists, not wired to logon |
| Depot filtering (OS/arch/language) | ✅ | ⚠️ | Applied in `info` output, not yet in download |
| UGC download | ✅ | ❌ | |

### Legend

- ✅ Fully implemented
- ⚠️ Partially implemented (flag exists but functionality incomplete)
- ❌ Not implemented

## steamroom-only Features

Features present in steamroom that are not available in DepotDownloader:

| Feature | Description |
|---|---|
| Pure Rust / static binary | No C dependencies, no system OpenSSL. Single static binary. |
| WebSocket CM transport | Connects via WSS in addition to TCP. More firewall-friendly. |
| HTTP/2 multiplexing | CDN chunk downloads use HTTP/2 connection multiplexing. |
| Chunk-level delta downloads | Compares Adler-32 checksums per chunk against existing files; only fetches changed chunks from CDN. |
| Non-atomic write mode | `--non-atomic` writes chunks directly to target files instead of staging + rename. |
| CDN server pool rotation | Lock-free round-robin server selection with per-server cooldown and Retry-After support. |
| Local Steam credential reuse | `--use-steam-token` extracts cached login tokens from Steam's local files (Windows DPAPI, Linux/macOS AES-256-CBC). |
| Local depot key reuse | `--local-keys` reads decryption keys from Steam's config.vdf instead of requesting from server. |
| Local config inspection | `local-info` command shows cached depot keys (with app name mapping) and beta branch hashes. |
| Manifest-only download | `save-manifest` command saves raw CDN response, decompressed manifest, and depot key without downloading content. |
| Local manifest reading | `files --manifest-file` reads a previously saved manifest offline, with auto-detected or explicit depot key. |
| Manifest diff | `diff` command compares two manifests and shows added, removed, and changed files. |
| Package queries | `packages` command queries Steam package (sub) details by ID. |
| Installing state tracking | Writes in-progress state to depot.json; interrupted downloads are visible and resumable. |
| Empty directory pruning | After delta updates, removes directories that became empty (only from paths in old manifest). |
| Progress bars | indicatif-based multi-progress with per-file and overall download tracking. |
| Quiet mode | `--quiet` suppresses all log output, keeping progress bars. `--no-progress` disables progress bars. |
| Mostly runtime-agnostic core | Protocol library uses `AsyncRead`/`AsyncWrite` traits; tokio is the current runtime but not deeply coupled. |
| Blocking thread pool for crypto | Chunk decrypt/decompress offloaded to `spawn_blocking`. |
| Serde KV deserializer | Deserialize Valve KeyValue format directly into typed Rust structs. |
| C/C++ FFI (Diplomat) | Generated C and C++ headers. Python example via nanobind. |
| Proto extraction tool | Extracts .proto definitions from steamclient64.dll (PE format). |
| Fuzz testing | cargo-fuzz targets with seed corpora for all binary format parsers. |
| `DD_COMPAT=1` mode | Flat-argument CLI compatible with original DepotDownloader. |
| `--raw` flag on `files` | Show encrypted filenames without attempting decryption. |
| `--raw-errors` flag | Show full Rust error chain for debugging. |
| `DD_DEVICE_NAME` env | Configurable device name for auth sessions. |
