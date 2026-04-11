# Fuzz Testing

Fuzz targets for steamroom parsers using `cargo-fuzz` (libfuzzer).

## Targets

| Target | Parser | Seed corpus |
|--------|--------|-------------|
| `fuzz_binary_kv` | Valve binary KeyValue format | AppState with string/int/uint64 fields |
| `fuzz_text_kv` | Valve text KeyValue format | AppState with nested depots |
| `fuzz_manifest` | Depot manifest (V4/V5 sections) | Minimal V4 manifest with payload+metadata |
| `fuzz_packet_header` | Steam CM packet header | Protobuf + simple header variants |
| `fuzz_frame` | VT01 TCP frame | Simple payload frame |

## Running locally (Linux or WSL)

```bash
# Build all targets
cargo +nightly fuzz build --fuzz-dir fuzz

# Run a specific target for 2 minutes
cargo +nightly fuzz run --fuzz-dir fuzz fuzz_binary_kv -- -max_total_time=120

# Run all targets (2 min each)
for t in fuzz_binary_kv fuzz_text_kv fuzz_manifest fuzz_packet_header fuzz_frame; do
  cargo +nightly fuzz run --fuzz-dir fuzz "$t" -- -max_total_time=120
done
```

## CI

In CI, run each fuzzer for 20 seconds as a smoke test:

```bash
for t in fuzz_binary_kv fuzz_text_kv fuzz_manifest fuzz_packet_header fuzz_frame; do
  cargo +nightly fuzz run --fuzz-dir fuzz "$t" -- -max_total_time=20
done
```

## Notes

- Requires nightly Rust (`cargo +nightly`)
- libfuzzer does not work on Windows MSVC (missing sanitizer DLLs); use Linux/WSL
- Corpus seeds are checked into `fuzz/corpus/`; generated corpus grows in the same directory
- Crash artifacts go to `fuzz/artifacts/` (gitignored)
