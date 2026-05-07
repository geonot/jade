//! FFI re-exports of selected runtime entry points so integration tests
//! can exercise the C runtime directly. Linking through `pub` items here
//! prevents the static archive from being dead-stripped.
//!
//! Not part of the user-visible API surface; only used by the Jade
//! crash-recovery and runtime smoke tests.

#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::ffi::c_void;
use std::os::raw::{c_char, c_int, c_uchar};

#[repr(C)]
pub struct CFile {
    _opaque: [u8; 0],
}

pub type WalReplayCb = extern "C" fn(
    op: c_uchar,
    payload: *const c_void,
    len: u32,
    ts: i64,
    user_data: *mut c_void,
);

unsafe extern "C" {
    pub fn jade_wal_open(path: *const c_char) -> *mut CFile;
    pub fn jade_wal_write(wal: *mut CFile, op: c_uchar, payload: *const c_void, len: u32);
    pub fn jade_wal_close(wal: *mut CFile);
    pub fn jade_wal_replay(wal: *mut CFile, cb: WalReplayCb, user_data: *mut c_void) -> i64;
    pub fn jade_wal_commit_group(wal: *mut CFile);
    pub fn jade_wal_size(wal: *mut CFile) -> i64;
    pub fn jade_wal_checkpoint(wal: *mut CFile);
}

/// Force linker to retain the WAL symbols by referencing each function
/// pointer. Called from a constructor in the test harness.
#[doc(hidden)]
pub fn force_link_wal() -> [usize; 7] {
    [
        jade_wal_open as usize,
        jade_wal_write as usize,
        jade_wal_close as usize,
        jade_wal_replay as usize,
        jade_wal_commit_group as usize,
        jade_wal_size as usize,
        jade_wal_checkpoint as usize,
    ]
}

// Suppress unused-import warnings for the C-int alias on platforms where
// future runtime additions may need it.
#[allow(dead_code)]
fn _force_use(_: c_int) {}
