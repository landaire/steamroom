use std::ffi::{c_char, CStr, CString};
use std::ptr;

static mut RUNTIME: Option<tokio::runtime::Runtime> = None;
static mut LAST_ERROR: Option<CString> = None;

#[no_mangle]
pub extern "C" fn steam_init() -> i32 {
    todo!()
}

#[no_mangle]
pub extern "C" fn steam_shutdown() {
    todo!()
}

#[no_mangle]
pub extern "C" fn steam_last_error() -> *const c_char {
    todo!()
}

#[no_mangle]
pub extern "C" fn steam_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            drop(CString::from_raw(s));
        }
    }
}
