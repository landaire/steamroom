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
| Depot key retrieval | ✅ | ✅ | |
| CDN server discovery | ✅ | ✅ | Service method RPC |
| CDN auth tokens | ✅ | ✅ | |
| Manifest request codes | ✅ | ✅ | |
| Manifest download + parsing (v5 protobuf) | ✅ | ✅ | |
| Manifest download + parsing (v4 binary) | ✅ | ✅ | Different magic values from v5 |
| Filename decryption (AES-256-ECB/CBC) | ✅ | ✅ | Handles line-wrapped base64 |
| Chunk download (HTTP) | ✅ | ✅ | Parallel with semaphore |
| Chunk decryption (AES-256) | ✅ | ✅ | ECB IV + CBC payload |
| Chunk decompression (PKZip) | ✅ | ✅ | |
| Chunk decompression (LZMA/VZip) | ✅ | ✅ | Valve LZMA with external uncompressed size |
| Checksum verification (Adler32 zero-seed) | ✅ | ✅ | Steam's non-standard zero seed |
| File verification (`--verify`) | ✅ | ✅ | SHA-1 + size match skips download |
| File list filtering (`--filelist`) | ✅ | ✅ | Case-insensitive matching |
| Regex file filtering | ✅ | ✅ | `--file-regex` |
| Manifest caching | ✅ | ✅ | `~/.depotdownloader/manifests/` |
| Download resumption (staging) | ✅ | ✅ | `.depotdownloader/staging/` with chunk-level resume |
| Depot config persistence | ✅ | ✅ | Tracks installed manifests for delta downloads |
| Workshop download | ✅ | ✅ | Full manifest + chunk pipeline |
| Branch selection | ✅ | ✅ | `--branch` flag |
| Encrypted manifest detection | ✅ | ✅ | Shown in `info` output |
| JSON output | ✅ | ✅ | `--format json` on info/manifests/files |
| Lancache detection + proxy | ✅ | ✅ | DNS-based detection |
| `--raw-errors` debug mode | ❌ | ✅ | Shows full error chain for troubleshooting |
| Configurable device name | ❌ | ✅ | `--device-name` / `DD_DEVICE_NAME` |
| Legacy CLI compat mode | ❌ | ✅ | `DD_COMPAT=1` flat-argument mode |
| C/C++ FFI bindings | ❌ | ✅ | Via Diplomat, with Python nanobind example |
| Serde deserializer for KV format | ❌ | ✅ | `#[derive(Deserialize)]` from PICS data |
| Proto extraction tool | ❌ | ✅ | Extracts .proto from steamclient64.dll |
| Fuzz testing | ❌ | ✅ | cargo-fuzz targets for all parsers |
| Account access verification (license check) | ✅ | ❌ | |
| Multi-depot file deduplication | ✅ | ❌ | |
| Custom login ID (`--loginid`) | ✅ | ⚠️ | CLI flag exists, not wired to logon |
| Depot filtering (OS/arch/language) | ✅ | ⚠️ | CLI flags exist, filtering not applied |
| Delta/differential chunk downloads | ✅ | ⚠️ | Config tracked, chunk diffing not implemented |
| Beta/branch passwords | ✅ | ⚠️ | CLI flag exists, password hash not implemented |
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
| Async runtime-agnostic core | Protocol library uses `futures` primitives, not tokio directly. |
| Blocking thread pool for crypto | Decrypt/decompress offloaded to blocking pool, matching Steam client's architecture. |
| Serde KV deserializer | Deserialize Valve KeyValue format directly into typed Rust structs. |
| C/C++ FFI (Diplomat) | Generated C and C++ headers. Python example via nanobind. |
| Proto extraction tool | Pure Rust tool extracts .proto definitions from PE binaries. |
| Fuzz testing | cargo-fuzz targets with seed corpora for all binary format parsers. |
| `DD_COMPAT=1` mode | Flat-argument CLI compatible with original DepotDownloader. |
| `--raw` flag on `files` | Show encrypted filenames without attempting decryption. |
| `--raw-errors` flag | Show full Rust error chain for debugging. |
| `DD_DEVICE_NAME` env | Configurable device name for auth sessions. |
