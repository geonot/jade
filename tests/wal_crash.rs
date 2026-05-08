//! Crash-recovery test for the WAL. Each test exercises the durability
//! contract advertised in `runtime/wal.c`: an entry that returns from
//! `jinn_wal_write` under the default `fdatasync` policy must survive a
//! `SIGKILL` of the writing process.

use std::ffi::{CString, c_void};
use std::path::PathBuf;

use jinnc::runtime_ffi::{
    force_link_wal, jinn_wal_close, jinn_wal_commit_group, jinn_wal_open, jinn_wal_replay,
    jinn_wal_write,
};

extern "C" fn count_cb(_op: u8, _payload: *const c_void, _len: u32, _ts: i64, ud: *mut c_void) {
    let counter = unsafe { &mut *(ud as *mut u64) };
    *counter += 1;
}

fn tmp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "jinn_wal_crash_{}_{}_{}.wal",
        std::process::id(),
        name,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    let _ = std::fs::remove_file(&p);
    p
}

fn ensure_linked() {
    std::hint::black_box(force_link_wal());
}

#[test]
fn wal_basic_durability_and_replay() {
    ensure_linked();
    unsafe { std::env::set_var("JINN_WAL_SYNC", "fdatasync") };
    let path = tmp_path("basic");
    let cpath = CString::new(path.to_str().unwrap()).unwrap();

    unsafe {
        let wal = jinn_wal_open(cpath.as_ptr());
        assert!(!wal.is_null(), "wal_open failed");
        for i in 0u32..50 {
            let payload = i.to_le_bytes();
            jinn_wal_write(wal, 1, payload.as_ptr() as *const c_void, 4);
        }
        jinn_wal_close(wal);

        let wal = jinn_wal_open(cpath.as_ptr());
        assert!(!wal.is_null());
        let mut count: u64 = 0;
        let n = jinn_wal_replay(wal, count_cb, &mut count as *mut u64 as *mut c_void);
        assert_eq!(n, 50);
        assert_eq!(count, 50);
        jinn_wal_close(wal);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn wal_group_commit_durability() {
    ensure_linked();
    unsafe { std::env::set_var("JINN_WAL_SYNC", "group") };
    let path = tmp_path("group");
    let cpath = CString::new(path.to_str().unwrap()).unwrap();

    unsafe {
        let wal = jinn_wal_open(cpath.as_ptr());
        assert!(!wal.is_null());
        for i in 0u32..100 {
            let payload = i.to_le_bytes();
            jinn_wal_write(wal, 1, payload.as_ptr() as *const c_void, 4);
        }
        jinn_wal_commit_group(wal);
        jinn_wal_close(wal);

        let wal = jinn_wal_open(cpath.as_ptr());
        assert!(!wal.is_null());
        let mut count: u64 = 0;
        let n = jinn_wal_replay(wal, count_cb, &mut count as *mut u64 as *mut c_void);
        assert_eq!(n, 100, "all records should survive group commit");
        jinn_wal_close(wal);
    }
    let _ = std::fs::remove_file(&path);
}

/// Worst-case crash test: child writes N records with fdatasync, then
/// SIGKILLs itself. Per the durability contract, all entries must
/// survive in the parent.
#[test]
fn wal_survives_kill_9() {
    ensure_linked();
    unsafe { std::env::set_var("JINN_WAL_SYNC", "fdatasync") };
    let path = tmp_path("kill9");
    let cpath = CString::new(path.to_str().unwrap()).unwrap();

    let pid = unsafe { libc_fork() };
    if pid == 0 {
        unsafe {
            let wal = jinn_wal_open(cpath.as_ptr());
            if wal.is_null() {
                libc_exit(2);
            }
            for i in 0u32..200 {
                let payload = i.to_le_bytes();
                jinn_wal_write(wal, 1, payload.as_ptr() as *const c_void, 4);
            }
            libc_kill(libc_getpid(), 9);
            libc_exit(1);
        }
    } else if pid < 0 {
        panic!("fork failed");
    } else {
        let mut status: i32 = 0;
        unsafe { libc_waitpid(pid, &mut status, 0) };
        unsafe {
            let wal = jinn_wal_open(cpath.as_ptr());
            assert!(!wal.is_null(), "wal_open failed in parent");
            let mut count: u64 = 0;
            let n = jinn_wal_replay(wal, count_cb, &mut count as *mut u64 as *mut c_void);
            assert_eq!(n, 200, "all fdatasync'd records must survive SIGKILL");
            assert_eq!(count, 200);
            jinn_wal_close(wal);
        }
        let _ = std::fs::remove_file(&path);
    }
}

unsafe extern "C" {
    #[link_name = "fork"]
    fn libc_fork() -> i32;
    #[link_name = "waitpid"]
    fn libc_waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    #[link_name = "_exit"]
    fn libc_exit(code: i32) -> !;
    #[link_name = "getpid"]
    fn libc_getpid() -> i32;
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}
