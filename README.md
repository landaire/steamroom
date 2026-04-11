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

Show app metadata, depots with sizes, and branches.

```bash
steamroom info --app 730

# Filter depots by OS
steamroom info --app 730 --os linux

# Include redistributable depots
steamroom info --app 730 --show-all

# JSON output for scripting
steamroom info --app 730 --format json
```

Example output:

```
App ID:  730
Name:    Counter-Strike 2
Type:    Game

Depots:
  ID        CONFIGURATION          SIZE          DL.
  2347770   64-bit            50.44 GiB    42.93 GiB
  2347771   Windows            7.07 GiB     5.05 GiB
  2347772   macOS              9.06 KiB     1.66 KiB
  2347773   Linux, 64-bit      6.62 GiB     4.71 GiB
  2347774   64-bit          1023.01 MiB   843.02 MiB
  ...

Branches:
  NAME               DESCRIPTION                           BUILD      TIME BUILT    TIME UPDATED
  animgraph_2_beta   Animgraph 2 Beta                      22720547   2d ago        2d ago
  1.41.4.1           1.41.4.1                              22627914   9d ago        9d ago
  public                                                   22627914   9d ago        9d ago
  1.41.4.0           1.41.4.0                              22370414   26d ago       26d ago
  ...
```

### `manifests`

List depot manifest IDs for a branch.

```bash
steamroom manifests --app 480
steamroom manifests --app 480 --branch previous
steamroom manifests --app 480 --format json
```

Example output:

```
Manifests for branch 'public':

  depot 229006   -> --
  depot 481      -> 3183503801510301321
```

### `files`

List files in a depot manifest.

```bash
steamroom files --app 480 --depot 481

# Plain output (one filename per line, for piping)
steamroom files --app 480 --depot 481 --format plain

# Raw byte sizes
steamroom files --app 480 --depot 481 --bytes

# JSON output
steamroom files --app 480 --depot 481 --format json
```

Example output:

```
Depot:    481
Manifest: 3183503801510301321
Created:  2019-02-06 21:51:33 UTC
Size:     1.82 MiB
Files:    8

FILENAME                          SIZE   CHUNKS
DejaVuSans.txt                2.76 KiB        1
sdkencryptedappticket.dll   558.28 KiB        1
DejaVuSans.ttf              703.96 KiB        1
installscript.vdf                514 B        1
steam_api.dll               219.78 KiB        1
SteamworksExample.exe       374.00 KiB        1
controller.vdf                1.53 KiB        1
D3D9VRDistort.cso                576 B        1
```

### `workshop`

Download Steam Workshop items.

```bash
steamroom workshop --app 440 --item 123456789 -o workshop/
```

## Authentication

steamroom supports multiple authentication methods:

| Method         | Flag                        | Notes                                            |
| -------------- | --------------------------- | ------------------------------------------------ |
| Anonymous      | (none)                      | Works for free games                             |
| Password       | `--username X --password Y` | Prompts if password omitted                      |
| Password + 2FA | `--username X`              | Prompts for guard code                           |
| QR code        | `--username X --qr`         | Scan with Steam mobile app                       |
| Saved token    | `--username X`              | Auto-loads from `~/.depotdownloader/tokens.json` |

Tokens are saved automatically after successful login and reused on subsequent runs.

## Legacy Compatibility

Set `DD_COMPAT=1` to use flat arguments compatible with the original DepotDownloader:

```bash
DD_COMPAT=1 steamroom --app 480 --depot 481 --dir output/ --verify
```

## Features

- TCP and WebSocket transports
- Pipelined chunk downloads with resume support
- Serde deserializer for Valve KeyValue format
- C/C++ FFI bindings via Diplomat, Python bindings via nanobind
- Proto extraction tool for `steamclient64.dll`
- Fuzz targets for binary parsers

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
| --------- | --------- | --------------- | ------- |

\\\\\\\ to: mpptwwzu 280b1323 "perf: better-optimized Adler32 implementation" (rebased revision)
-| App info query (480) | 0.83s ± 0.21s | 2.28s ± 1.17s | **2.7x** |
-| File listing (480/481) | 0.81s ± 0.02s | 1.05s ± 0.18s | **1.3x** |
-| Download Spacewar (1.8 MB) | 1.80s ± 0.12s | 4.03s ± 0.16s | **2.2x** |
-| Download CS2 content (2.5 GB) | 22.0s ± 0.5s | 33.1s ± 2.2s | **1.5x** |
+| App info query (480) | 0.67s ± 0.06s | 2.85s ± 1.02s | **4.3x** |
+| File listing (480/481) | 1.67s ± 0.06s | 3.34s ± 1.01s | **2.0x** |
+| Download Spacewar (1.8 MB) | 1.23s ± 0.14s | 4.04s ± 0.16s | **3.3x** |
+| Download CS2 content (2.5 GB) | 23.6s | 32.1s | **1.4x** |

Both tools are network-bound for large downloads. Results will vary by network and hardware. Run `bench/run.sh` to reproduce on your own setup.

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
