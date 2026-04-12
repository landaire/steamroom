//! Offline profiling harness for steamroom subsystems.
//!
//! Exercises each hot path in a tight loop with realistic data sizes so you
//! can profile them independently with Instruments / perf / etc.
//!
//! Usage:
//!   cargo build --release -p steamroom-cli --bin bench-profile
//!   xcrun xctrace record --template 'Time Profiler' --launch -- \
//!       target/release/bench-profile [BENCH_NAME]
//!
//! Available benchmarks:
//!   all              Run all benchmarks sequentially (default)
//!   aes-decrypt      AES-256-CBC decryption (1 MB blocks)
//!   aes-encrypt      AES-256-CBC encryption (1 MB blocks)
//!   chunk-process    Full chunk pipeline (decrypt + checksum, no compression)
//!   checksum-adler   SteamAdler32 over 1 MB
//!   checksum-sha1    SHA1 over 1 MB
//!   kv-parse         Binary KeyValue parsing (200 keys)

use std::hint::black_box;
use std::time::Instant;

use steamroom::crypto;
use steamroom::depot::DepotKey;
use steamroom::depot::chunk;
use steamroom::types::key_value::KeyValue;
use steamroom::util::checksum::Sha1Hash;
use steamroom::util::checksum::SteamAdler32;

const ITERATIONS: u32 = 10_000;
const CHUNK_ITERATIONS: u32 = 5_000;

fn main() {
    let bench = std::env::args().nth(1).unwrap_or_else(|| "all".into());

    let key = [0xAAu8; 32];
    let iv = [0x42u8; 16];

    // 1 MB plaintext (realistic chunk size)
    let plaintext: Vec<u8> = (0..1_048_576u32).map(|i| (i % 251) as u8).collect();
    let ciphertext = crypto::symmetric_encrypt_cbc(&plaintext, &key, &iv).unwrap();

    // Encrypted chunk: ECB(IV) || CBC(plaintext)
    let encrypted_iv = crypto::symmetric_encrypt_ecb_nopad(&iv, &key).unwrap();
    let depot_key = DepotKey(key);
    let checksum = SteamAdler32::compute(&plaintext);
    let mut chunk_data = Vec::with_capacity(encrypted_iv.len() + ciphertext.len());
    chunk_data.extend_from_slice(&encrypted_iv);
    chunk_data.extend_from_slice(&ciphertext);

    // Binary KV data
    let kv_data = build_test_kv(200);

    let benches: &[(&str, &dyn Fn())] = &[
        ("aes-decrypt", &|| {
            black_box(crypto::symmetric_decrypt_cbc(&ciphertext, &key, &iv).unwrap());
        }),
        ("aes-encrypt", &|| {
            black_box(crypto::symmetric_encrypt_cbc(&plaintext, &key, &iv).unwrap());
        }),
        ("chunk-process", &|| {
            black_box(
                chunk::process_chunk(&chunk_data, &depot_key, plaintext.len() as u32, checksum.0)
                    .unwrap(),
            );
        }),
        ("checksum-adler", &|| {
            black_box(SteamAdler32::compute(&plaintext));
        }),
        ("checksum-sha1", &|| {
            black_box(Sha1Hash::compute(&plaintext));
        }),
        ("kv-parse", &|| {
            black_box(KeyValue::from_binary(&kv_data).unwrap());
        }),
    ];

    let selected: Vec<_> = if bench == "all" {
        benches.iter().collect()
    } else {
        match benches.iter().find(|(name, _)| *name == bench) {
            Some(b) => vec![b],
            None => {
                eprintln!("unknown benchmark: {bench}");
                eprintln!(
                    "available: all, {}",
                    benches
                        .iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                std::process::exit(1);
            }
        }
    };

    for (name, func) in &selected {
        let iters = if name.contains("chunk") {
            CHUNK_ITERATIONS
        } else {
            ITERATIONS
        };
        run_bench(name, iters, func);
    }
}

fn run_bench(name: &str, iterations: u32, f: &dyn Fn()) {
    // Warmup
    for _ in 0..10 {
        f();
    }

    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = start.elapsed();
    let per_iter = elapsed / iterations;

    eprintln!("{name:20} {iterations:>7} iters  {elapsed:>10.3?} total  {per_iter:>10.3?}/iter");
}

/// Build binary KV data with `n` string keys + nested subsection.
fn build_test_kv(n: usize) -> Vec<u8> {
    let mut data = Vec::new();
    data.push(0u8); // subsection tag
    data.extend_from_slice(b"AppState\0");

    for i in 0..n {
        data.push(1u8); // string tag
        data.extend_from_slice(format!("key_{i:04}\0").as_bytes());
        data.extend_from_slice(format!("value_{i:04}\0").as_bytes());
    }

    for i in 0..n / 4 {
        data.push(2u8); // int32 tag
        data.extend_from_slice(format!("num_{i:04}\0").as_bytes());
        data.extend_from_slice(&(i as i32).to_le_bytes());
    }

    data.push(0u8); // nested subsection
    data.extend_from_slice(b"UserConfig\0");
    for i in 0..n / 2 {
        data.push(1u8);
        data.extend_from_slice(format!("opt_{i:03}\0").as_bytes());
        data.extend_from_slice(b"enabled\0");
    }
    data.push(8u8); // end nested
    data.push(8u8); // end root
    data
}
