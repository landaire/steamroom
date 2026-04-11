#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = steamroom::depot::manifest::DepotManifest::parse(data);
});
