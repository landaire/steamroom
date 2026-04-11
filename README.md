# steamroom

Utilities for interacting with Steam's API

## History

This project is a **cleanroom** reimplementation of [DepotDownloader](https://github.com/steamre/depotdownloader).

I originally used an LLM to translate DepotDownloader to Rust, and put all of that [DepotDownloader-rs](https://github.com/landaire/depotdownloader-rs). However, I realized that GPL licensing is a pain in the ass for Rust projects because of static linking, and decided to do the following:

1. Generate docs for the conversion library
2. Delete the source code from the docs
3. Copy that + the file tree and old `ddl` binary to a new repo
4. Instruct a new LLM session how to reverse engineer steam (using Binary Ninja MCP + Steam libs loaded)
5. Told it to reimplement it to the API spec
6. ???
7. 4 Hours later, we're GPL-free

Any major improvements done to this library should, in spirit of collaboration, be shared back to the SteamRE/DepotDownloader project in the spirit of advancing everyone's capabilities.

Not to air my personal grievances with GPL in this README, but DepotDownloader has been a godsend for many projects and I do believe in the spirit of upstreaming changes you make to libraries you use. I don't like the idea of GPL infecting things which statically link against the library, however. And that is the only reason why this library exists as a cleanroom reimplementation.

## Install

```bash
cargo install --path crates/steamroom-cli
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

JSON output (`--format json`) returns the raw PICS app info as JSON, useful for scripting.

### `manifests`

List depot manifests for a branch.

```bash
steamroom manifests --app 480
steamroom manifests --app 480 --branch previous
steamroom manifests --app 480 --format json
```

Example output:
```
481	3183503801510301321
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

Example output:
```
DejaVuSans.txt              2.8 KB
sdkencryptedappticket.dll   558.3 KB
DejaVuSans.ttf              704.0 KB
installscript.vdf           514 B
steam_api.dll               219.8 KB
SteamworksExample.exe       374.0 KB
controller.vdf              1.5 KB
D3D9VRDistort.cso           576 B
```

Plain output (one filename per line, for piping):
```
DejaVuSans.txt
sdkencryptedappticket.dll
DejaVuSans.ttf
installscript.vdf
steam_api.dll
SteamworksExample.exe
controller.vdf
D3D9VRDistort.cso
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
- **Pipelined downloads** — network fetch and CPU-bound decrypt/decompress run concurrently via a channel-based pipeline, matching Steam client's multi-pool architecture
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

Compared against [DepotDownloader](https://github.com/SteamRE/DepotDownloader) v3.4.0 (C#/.NET) using [hyperfine](https://github.com/sharkdp/hyperfine). Anonymous login, Windows 11, same network.

| Benchmark | steamroom | DepotDownloader | Speedup |
|-----------|-----------|-----------------|---------|
| App info query (480) | 1.15s ± 0.09s | 2.37s ± 0.73s | **2.1x** |
| File listing (480/481) | 0.88s ± 0.10s | 2.79s ± 1.14s | **3.2x** |
| Download Spacewar (1.8 MB) | 1.75s ± 0.56s | 3.61s ± 1.18s | **2.1x** |
| Download CS2 maps (1.7 GB) | 18.4s ± 0.2s | 18.0s ± 0.2s | **1.0x** |

steamroom is significantly faster for metadata operations and small downloads due to lower startup overhead (no .NET runtime), pipelined chunk processing, and a more efficient connection setup. Both tools are network-bound for large downloads.

<details>
<summary>Reproduce benchmarks</summary>

```bash
# Build release
cargo build --release -p steamroom-cli

# Run with hyperfine (clean state each run to prevent resume skew)
hyperfine --min-runs 3 -N \
  --prepare "rm -rf /tmp/sr /tmp/dd" \
  -n steamroom "steamroom download --app 480 --depot 481 -o /tmp/sr" \
  -n DepotDownloader "DepotDownloader -app 480 -depot 481 -dir /tmp/dd"
```

Or use the included benchmark script with nix:
```bash
nix develop
./bench/run.sh /path/to/scratch
```
</details>

See [FEATURES.md](FEATURES.md) for a full feature comparison.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
