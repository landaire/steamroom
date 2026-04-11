#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = steamroom::connection::framing::Frame::decode(data);
});
