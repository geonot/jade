#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::ffi::c_void;
use std::os::raw::{c_char, c_int, c_uchar};

#[repr(C)]
pub struct CFile {
    _opaque: [u8; 0],
}

pub type WalReplayCb =
    extern "C" fn(op: c_uchar, payload: *const c_void, len: u32, ts: i64, user_data: *mut c_void);

unsafe extern "C" {
    pub fn jinn_wal_open(path: *const c_char) -> *mut CFile;
    pub fn jinn_wal_write(wal: *mut CFile, op: c_uchar, payload: *const c_void, len: u32);
    pub fn jinn_wal_close(wal: *mut CFile);
    pub fn jinn_wal_replay(wal: *mut CFile, cb: WalReplayCb, user_data: *mut c_void) -> i64;
    pub fn jinn_wal_commit_group(wal: *mut CFile);
    pub fn jinn_wal_size(wal: *mut CFile) -> i64;
    pub fn jinn_wal_checkpoint(wal: *mut CFile);
}

#[doc(hidden)]
pub fn force_link_wal() -> [usize; 7] {
    [
        jinn_wal_open as usize,
        jinn_wal_write as usize,
        jinn_wal_close as usize,
        jinn_wal_replay as usize,
        jinn_wal_commit_group as usize,
        jinn_wal_size as usize,
        jinn_wal_checkpoint as usize,
    ]
}

unsafe extern "C" {
    pub fn jinn_chan_create(elem_size: usize, capacity: usize) -> *mut c_void;
    pub fn jinn_chan_send(ch: *mut c_void, data: *const c_void);
    pub fn jinn_chan_recv(ch: *mut c_void, data_out: *mut c_void) -> c_int;
    pub fn jinn_chan_close(ch: *mut c_void);
    pub fn jinn_chan_destroy(ch: *mut c_void);
}

#[doc(hidden)]
pub fn force_link_chan() -> [usize; 5] {
    [
        jinn_chan_create as usize,
        jinn_chan_send as usize,
        jinn_chan_recv as usize,
        jinn_chan_close as usize,
        jinn_chan_destroy as usize,
    ]
}
