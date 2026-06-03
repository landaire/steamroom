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

### `save-manifest`

Download and save a depot manifest without downloading content. Saves the raw CDN response (`.zip`), decompressed manifest (`.manifest`), and depot key (`depot.json`).

```bash
# Save latest manifest
steamroom save-manifest --app 552990 --depot 552993 -o manifests/

# Save a specific manifest version
steamroom save-manifest --app 552990 --depot 552993 --manifest 6791242355553147175 -o manifests/
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

# Read from a local manifest file (key auto-detected from depot.json)
steamroom files --manifest-file manifests/.depotdownloader/manifests/552993_123.manifest --depot 552993

# Explicit depot key (hex)
steamroom files --manifest-file path/to/552993_123.manifest --depot-key a4ac589393a2c8679c1f99b2e6fcee10554c030a0a6bf69bd4e22a13da7aab55

# Show raw encrypted filenames (no key needed)
steamroom files --manifest-file path/to/552993_123.manifest --raw
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

### `diff`

Compare two manifests and show added, removed, and changed files.

```bash
steamroom diff --app 480 --depot 481 --from 3183503801510301321 --to 8271816315493618441
steamroom diff --app 480 --depot 481 --from 3183503801510301321 --to 8271816315493618441 --format json
```

### `packages`

Query Steam package (sub) details by ID.

```bash
steamroom packages 17906
steamroom packages 17906 29197 --format json
```

### `workshop`

Download Steam Workshop items.

```bash
steamroom workshop --app 440 --item 123456789 -o workshop/
```

### `local-info`

Show locally cached depot keys and beta branches from Steam's config.vdf.

```bash
steamroom local-info
steamroom local-info --format json
steamroom local-info --users
steamroom local-info --user myaccount
```

## Daemon mode

`steamroom` can run as a background daemon that holds an authenticated Steam session and
services CLI invocations over a local socket. Reusing the session avoids the per-command
CM discovery, handshake, and logon round trip, so repeated commands run faster.

The daemon authenticates either at launch, when `daemon start` is given auth flags, or
lazily on its first job, when started without them (using an auto-detected saved token, or
anonymous). It serves one account for its lifetime; to switch accounts, stop and restart
it. It reconnects if the CM connection drops.

```bash
# Start a daemon. With auth flags it logs in now; without them it logs in
# lazily on the first job.
steamroom --username myaccount daemon start
steamroom daemon start

# Route commands through the daemon. If none is running, --use-daemon
# starts one in lazy auth mode first.
steamroom --use-daemon info --app 730
steamroom --use-daemon download --app 480 --depot 481 -o spacewar/

# Jump the queue.
steamroom --use-daemon --priority info --app 730

# Submit without waiting, then reattach by job id.
steamroom --use-daemon --detach download --app 480 --depot 481
steamroom daemon attach <job-id>

# Observe.
steamroom daemon status              # ratatui dashboard
steamroom daemon status --text       # one-shot text snapshot
steamroom daemon status --format json
steamroom daemon info                # pid, socket, and stop command

# Stop.
steamroom daemon stop
steamroom daemon stop --force        # cancel the active job too
```

Job history persists across restarts, so `daemon status` and `daemon attach` can see jobs
from earlier daemon runs.

Auth flags (`--username`, `--password`, `--qr`, `--use-steam-token`, `--remember-password`,
`--device-name`) and `--capture` are ignored, with a warning, under `--use-daemon`; the
daemon serves the account it authenticated as. Set them on `daemon start` instead.

## Authentication

steamroom supports multiple authentication methods:

| Method         | Flag                        | Notes                                            |
| -------------- | --------------------------- | ------------------------------------------------ |
| Anonymous      | (none)                      | Works for free games                             |
| Password       | `--username X --password Y` | Prompts if password omitted                      |
| Password + 2FA | `--username X`              | Prompts for guard code                           |
| QR code        | `--username X --qr`         | Scan with Steam mobile app                       |
| Saved token    | `--username X`              | Auto-loads from `~/.depotdownloader/tokens.json` |
| Steam token    | `--use-steam-token`         | Reuses cached token from local Steam installation |

Tokens are saved automatically after successful login and reused on subsequent runs.

## Legacy Compatibility

Set `DD_COMPAT=1` to use single-dash arguments compatible with the original DepotDownloader:

```bash
DD_COMPAT=1 steamroom -app 480 -depot 481 -dir output/ -validate
```

## Features

- TCP and WebSocket CM transports
- HTTP/2 multiplexed chunk downloads with CDN server pool rotation
- Chunk-level delta downloads (only fetches changed chunks via Adler-32 comparison)
- Local Steam credential reuse (`--use-steam-token`) on Windows, Linux, and macOS
- Local depot key and beta hash reuse from Steam's config.vdf (`--local-keys`)
- Manifest diff, package queries, manifest-only download, local manifest reading
- Non-atomic write mode for direct chunk-to-file writes
- Serde deserializer for Valve KeyValue format
- C/C++ FFI bindings via Diplomat, Python bindings via nanobind
- Proto extraction tool for steamclient64.dll
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

| Benchmark                     | steamroom     | DepotDownloader | Speedup  |
| ----------------------------- | ------------- | --------------- | -------- |
| App info query (480)          | 0.67s ± 0.06s | 2.85s ± 1.02s   | **4.3x** |
| File listing (480/481)        | 1.67s ± 0.06s | 3.34s ± 1.01s   | **2.0x** |
| Download Spacewar (1.8 MB)    | 1.23s ± 0.14s | 4.04s ± 0.16s   | **3.3x** |
| Download CS2 content (2.5 GB) | 23.6s         | 32.1s           | **1.4x** |

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
