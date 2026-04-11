# steamroom

A clean room, MIT/Apache-2.0 licensed Rust implementation of a Steam depot downloader.

Downloads game content from Steam's CDN using the Steam client protocol. Supports anonymous and authenticated access, encrypted depots, workshop items, and all compression formats used by Steam.

## Install

```bash
cargo install --path crates/depotdownloader
```

## Quick Start

```bash
# Download Spacewar (free game, no login required)
steamroom download --app 480 --depot 481 -o spacewar/

# Download with authentication (prompts for password + 2FA)
steamroom --username myaccount download --app 440 -o tf2/

# QR code login (scan with Steam mobile app)
steamroom --username myaccount --qr download --app 440 -o tf2/

# Use a saved token (auto-detected from ~/.depotdownloader/tokens.json)
steamroom --username myaccount download --app 440 -o tf2/
```

## Commands

### `download`

Download depot content to a local directory.

```bash
# Basic download
steamroom download --app 480 --depot 481 -o output/

# Specific manifest version
steamroom download --app 480 --depot 481 --manifest 3183503801510301321 -o output/

# Download a specific branch
steamroom download --app 480 --depot 481 --branch previous -o output/

# Filter files by regex
steamroom download --app 480 --depot 481 --file-regex '\.dll$' -o output/

# Filter files by list
steamroom download --app 480 --depot 481 --filelist files.txt -o output/

# Verify existing files (skip up-to-date, re-download changed)
steamroom download --app 480 --depot 481 --verify -o output/

# Control parallelism
steamroom download --app 480 --depot 481 --max-downloads 16 -o output/
```

### `info`

Show app metadata: name, type, depots, branches, encrypted manifests.

```bash
steamroom info --app 480

# JSON output for scripting
steamroom info --app 480 --format json
```

Example output:
```
App ID:  480
Name:    Spacewar
Type:    Game

Depots (2):
  229006:
  481:

Branches:
  previous: build 316058 updated 1503510482 (SDK 1.30)
  public: build 3538192 updated 1549489971
```

### `manifests`

List depot manifests for a branch.

```bash
steamroom manifests --app 480
steamroom manifests --app 480 --branch previous
steamroom manifests --app 480 --format json
```

### `files`

List files in a depot manifest.

```bash
steamroom files --app 480 --depot 481

# Plain output (one filename per line, for piping)
steamroom files --app 480 --depot 481 --format plain

# JSON output
steamroom files --app 480 --depot 481 --format json

# Show raw encrypted filenames
steamroom files --app 480 --depot 481 --raw
```

### `workshop`

Download Steam Workshop items.

```bash
steamroom workshop --app 440 --item 123456789 -o workshop/
```

## Authentication

steamroom supports multiple authentication methods:

| Method | Flag | Notes |
|--------|------|-------|
| Anonymous | (none) | Works for free games |
| Password | `--username X --password Y` | Prompts if password omitted |
| Password + 2FA | `--username X` | Prompts for guard code |
| QR code | `--username X --qr` | Scan with Steam mobile app |
| Saved token | `--username X` | Auto-loads from `~/.depotdownloader/tokens.json` |

Tokens are saved automatically after successful login and reused on subsequent runs.

## Legacy Compatibility

Set `DD_COMPAT=1` to use flat arguments compatible with the original DepotDownloader:

```bash
DD_COMPAT=1 steamroom --app 480 --depot 481 --dir output/ --verify
```

## Unique Features

- **Pure Rust** — no C dependencies, no system OpenSSL, fully static binary
- **Dual transport** — TCP (with custom session cipher) and WebSocket (TLS) to Steam CM servers
- **Async runtime-agnostic core** — the `steamroom` protocol library uses `futures` primitives, not tokio directly
- **Parallel chunk downloads** — semaphore-bounded concurrency with CPU-bound decrypt/decompress offloaded to a blocking thread pool (matching Steam client's architecture)
- **Download resumption** — staging files survive interrupts, chunk-level resume on restart
- **Serde deserializer for Valve KV** — `#[derive(Deserialize)]` your structs and deserialize directly from PICS data
- **C/C++ FFI via Diplomat** — generated C and C++ headers, Python bindings via nanobind
- **Proto extraction tool** — extracts `.proto` definitions from `steamclient64.dll` using pure Rust PE parser
- **Fuzz testing** — cargo-fuzz targets with seed corpora for all binary parsers

## Architecture

```
steamroom              — Core Steam protocol: crypto, connection, transport, depot, messages
steamroom-client       — High-level: download orchestration, manifest cache, credentials
steamroom-cli          — CLI binary (produces `steamroom` executable)
steamroom-ffi          — C/C++ FFI bindings via Diplomat
steamroom-proto-extract — Tool to extract protobuf definitions from Steam binaries
```

## Benchmarks

Benchmarks compare steamroom against DepotDownloader (C#) using [hyperfine](https://github.com/sharkdp/hyperfine). Run inside the nix dev shell:

```bash
# Enter dev shell (provides rust, hyperfine, dotnet for DepotDownloader)
nix develop

# Build steamroom in release mode
cargo build --release -p steamroom-cli

# Install DepotDownloader
dotnet tool install -g DepotDownloader

# Run benchmarks (scratch dir on a drive with space)
./bench/run.sh /mnt/g/tmp/steamroom-bench
```

### Test Matrix

| Test | What it measures | Data size |
|------|-----------------|-----------|
| `info` | Login + PICS query latency | — |
| `files` | Manifest fetch + parse + filename decrypt | — |
| `spacewar` | Full download pipeline (small) | ~1.8 MB |
| `cs2` | Full download pipeline (large, DLL subset) | ~2 GB |

Each download test cleans the output directory before every run to prevent resume from skewing results. Results are saved as JSON in `$SCRATCH/results/`.

See [FEATURES.md](FEATURES.md) for a full feature comparison.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
