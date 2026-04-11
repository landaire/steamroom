#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    let _ = steamroom::types::key_value::parse_text_kv(data);
});
