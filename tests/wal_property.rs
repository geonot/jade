//! Property tests for the Write-Ahead Log (P1-14).
//!
//! The WAL is the durability substrate for the persistent store. Its single
//! most important contract under crash conditions is **prefix consistency**:
//!
//!   After a crash at *any* point — a torn tail write, a truncation in the
//!   middle of an entry, or a corrupted byte — replay must return a byte-exact
//!   *prefix* of the entries that were written. It may never resurrect a
//!   partially-written entry as if it were committed, never reorder entries,
//!   and never surface an entry whose contents differ from what was appended.
//!
//! These tests drive the real C runtime WAL (`runtime/wal.c`) through its FFI
//! surface with randomized operation sequences and randomized crash points,
//! then assert the prefix invariant exactly. The on-disk entry layout is:
//!
//!   [4B payload_len][1B op][8B timestamp][payload_len bytes][4B CRC32]
//!
//! so a fully-present entry occupies `17 + payload_len` bytes, and replay's
//! header/length/CRC checks must reject anything less than a complete entry.
//!
//! CI runs this with a large `PROPTEST_CASES` (≈5 min budget); locally it runs
//! the proptest default (256 cases) and is fast.

use std::ffi::{CString, c_void};
use std::os::raw::c_uchar;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use jinnc::runtime_ffi::{
    force_link_wal, jinn_wal_close, jinn_wal_open, jinn_wal_replay, jinn_wal_write,
};
use proptest::prelude::*;

/// On-disk size of one entry: 4B len + 1B op + 8B ts + payload + 4B CRC.
const ENTRY_OVERHEAD: u64 = 4 + 1 + 8 + 4;
/// Size of the WAL magic header that precedes all entries.
const MAGIC_LEN: u64 = 8;

fn ensure_linked() {
    std::hint::black_box(force_link_wal());
    // The WAL caches its sync policy on first open (process-global static).
    // This is a separate test binary from wal_crash.rs, so we own that static
    // and pin "none": these tests simulate crashes by editing the file bytes
    // after a clean close, so per-entry fsync is pure overhead here.
    unsafe { std::env::set_var("JINN_WAL_SYNC", "none") };
}

fn unique_path(tag: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "jinn_wal_prop_{}_{}_{}.wal",
        std::process::id(),
        tag,
        n
    ));
    let _ = std::fs::remove_file(&p);
    p
}

struct Collector {
    entries: Vec<(u8, Vec<u8>)>,
}

extern "C" fn collect_cb(op: c_uchar, payload: *const c_void, len: u32, _ts: i64, ud: *mut c_void) {
    let col = unsafe { &mut *(ud as *mut Collector) };
    let bytes = if len > 0 && !payload.is_null() {
        unsafe { std::slice::from_raw_parts(payload as *const u8, len as usize).to_vec() }
    } else {
        Vec::new()
    };
    col.entries.push((op, bytes));
}

/// Append `ops` to a fresh WAL and close it cleanly (libc buffers flushed to
/// the kernel; the bytes are now what a crash would leave behind).
fn write_wal(path: &Path, ops: &[(u8, Vec<u8>)]) {
    let cpath = CString::new(path.to_str().unwrap()).unwrap();
    unsafe {
        let wal = jinn_wal_open(cpath.as_ptr());
        assert!(!wal.is_null(), "wal_open failed for {path:?}");
        for (op, payload) in ops {
            let ptr = if payload.is_empty() {
                std::ptr::null()
            } else {
                payload.as_ptr() as *const c_void
            };
            jinn_wal_write(wal, *op, ptr, payload.len() as u32);
        }
        jinn_wal_close(wal);
    }
}

/// Reopen the WAL and replay it, collecting every entry the runtime accepts.
fn replay_wal(path: &Path) -> Vec<(u8, Vec<u8>)> {
    let cpath = CString::new(path.to_str().unwrap()).unwrap();
    let mut col = Collector {
        entries: Vec::new(),
    };
    unsafe {
        let wal = jinn_wal_open(cpath.as_ptr());
        assert!(!wal.is_null(), "wal_open(replay) failed for {path:?}");
        jinn_wal_replay(wal, collect_cb, &mut col as *mut Collector as *mut c_void);
        jinn_wal_close(wal);
    }
    col.entries
}

/// Cumulative end offsets of each entry on disk, including the magic header.
/// `end_offset[i]` is the first byte *after* entry `i`.
fn end_offsets(ops: &[(u8, Vec<u8>)]) -> Vec<u64> {
    let mut off = MAGIC_LEN;
    ops.iter()
        .map(|(_, p)| {
            off += ENTRY_OVERHEAD + p.len() as u64;
            off
        })
        .collect()
}

/// A randomized, valid WAL operation: op in 1..=4 (insert/update/delete/destroy)
/// with a small arbitrary payload (delete/destroy carry offset bytes; empty
/// payloads are legal too).
fn op_strategy() -> impl Strategy<Value = (u8, Vec<u8>)> {
    (1u8..=4, prop::collection::vec(any::<u8>(), 0..=48))
}

fn ops_strategy() -> impl Strategy<Value = Vec<(u8, Vec<u8>)>> {
    prop::collection::vec(op_strategy(), 1..=40)
}

proptest! {
    /// Clean round-trip: with no crash, replay returns exactly what was written,
    /// in order, byte-for-byte. This is the baseline the crash properties refine.
    #[test]
    fn replay_roundtrips_cleanly(ops in ops_strategy()) {
        ensure_linked();
        let path = unique_path("roundtrip");
        write_wal(&path, &ops);
        let got = replay_wal(&path);
        let _ = std::fs::remove_file(&path);
        prop_assert_eq!(got, ops);
    }

    /// Crash via truncation at an arbitrary byte: a torn tail write, or a power
    /// loss that left the file short. Replay must return exactly the maximal
    /// prefix of entries that are *fully* present on disk — never a partial
    /// entry, never garbage past the cut.
    #[test]
    fn truncation_yields_exact_prefix(
        ops in ops_strategy(),
        cut_permille in 0u64..=1000,
    ) {
        ensure_linked();
        let path = unique_path("trunc");
        write_wal(&path, &ops);

        let ends = end_offsets(&ops);
        let full_len = *ends.last().unwrap();
        let entries_bytes = full_len - MAGIC_LEN;

        // Choose a truncation length in [MAGIC_LEN, full_len]. Keep the magic
        // header intact (a WAL without magic is a *different* failure mode —
        // jinn_wal_open recreates it — exercised elsewhere).
        let cut_len = MAGIC_LEN + (entries_bytes * cut_permille) / 1000;
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(cut_len).unwrap();
        drop(f);

        // Expected survivors: every entry whose end offset fits within cut_len.
        let expected_k = ends.iter().filter(|&&e| e <= cut_len).count();

        let got = replay_wal(&path);
        let _ = std::fs::remove_file(&path);

        prop_assert_eq!(
            got.len(), expected_k,
            "cut_len={} full_len={} expected {} survivors, got {}",
            cut_len, full_len, expected_k, got.len()
        );
        prop_assert_eq!(&got[..], &ops[..expected_k]);
    }

    /// Crash via a corrupted byte inside an entry's CRC-covered region (op,
    /// timestamp, or payload — never the length prefix, never the stored CRC).
    /// The CRC check must catch it: every entry before the damaged one replays
    /// intact, and replay stops exactly at the damaged entry.
    #[test]
    fn corruption_stops_at_damaged_entry(
        ops in ops_strategy(),
        which in any::<u64>(),
        byte_sel in any::<u64>(),
    ) {
        ensure_linked();
        let n = ops.len() as u64;
        let j = (which % n) as usize;
        let path = unique_path("corrupt");
        write_wal(&path, &ops);

        // Locate entry j's CRC-covered bytes: [entry_start+4 .. entry_start+13+plen).
        // That is the 1B op + 8B timestamp + payload — exactly the bytes the
        // stored CRC32 protects. The 4B length prefix and 4B trailing CRC are
        // deliberately excluded so the damage can only manifest as a CRC
        // mismatch (no length realignment, no accidental CRC rewrite).
        let ends = end_offsets(&ops);
        let entry_start = if j == 0 { MAGIC_LEN } else { ends[j - 1] };
        let plen = ops[j].1.len() as u64;
        let region_start = entry_start + 4;          // skip the length prefix
        let region_len = 1 + 8 + plen;               // op + ts + payload
        let flip_at = region_start + (byte_sel % region_len);

        let mut bytes = std::fs::read(&path).unwrap();
        bytes[flip_at as usize] ^= 0xFF;             // guaranteed value change
        std::fs::write(&path, &bytes).unwrap();

        let got = replay_wal(&path);
        let _ = std::fs::remove_file(&path);

        // Entries before j are untouched and replay; entry j fails its CRC and
        // halts replay. Result is the exact prefix original[..j].
        prop_assert_eq!(got.len(), j, "damage in entry {} should stop replay there", j);
        prop_assert_eq!(&got[..], &ops[..j]);
    }
}
