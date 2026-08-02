use std::ffi::{CString, c_void};
use std::os::raw::c_uchar;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use jinnc::runtime_ffi::{
    force_link_wal, jinn_wal_close, jinn_wal_open, jinn_wal_replay, jinn_wal_write,
};
use proptest::prelude::*;

const ENTRY_OVERHEAD: u64 = 4 + 1 + 8 + 4;

const MAGIC_LEN: u64 = 8;

fn ensure_linked() {
    std::hint::black_box(force_link_wal());

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

fn end_offsets(ops: &[(u8, Vec<u8>)]) -> Vec<u64> {
    let mut off = MAGIC_LEN;
    ops.iter()
        .map(|(_, p)| {
            off += ENTRY_OVERHEAD + p.len() as u64;
            off
        })
        .collect()
}

fn op_strategy() -> impl Strategy<Value = (u8, Vec<u8>)> {
    (1u8..=4, prop::collection::vec(any::<u8>(), 0..=48))
}

fn ops_strategy() -> impl Strategy<Value = Vec<(u8, Vec<u8>)>> {
    prop::collection::vec(op_strategy(), 1..=40)
}

proptest! {


    #[test]
    fn replay_roundtrips_cleanly(ops in ops_strategy()) {
        ensure_linked();
        let path = unique_path("roundtrip");
        write_wal(&path, &ops);
        let got = replay_wal(&path);
        let _ = std::fs::remove_file(&path);
        prop_assert_eq!(got, ops);
    }





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




        let cut_len = MAGIC_LEN + (entries_bytes * cut_permille) / 1000;
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(cut_len).unwrap();
        drop(f);


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






        let ends = end_offsets(&ops);
        let entry_start = if j == 0 { MAGIC_LEN } else { ends[j - 1] };
        let plen = ops[j].1.len() as u64;
        let region_start = entry_start + 4;
        let region_len = 1 + 8 + plen;
        let flip_at = region_start + (byte_sel % region_len);

        let mut bytes = std::fs::read(&path).unwrap();
        bytes[flip_at as usize] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();

        let got = replay_wal(&path);
        let _ = std::fs::remove_file(&path);



        prop_assert_eq!(got.len(), j, "damage in entry {} should stop replay there", j);
        prop_assert_eq!(&got[..], &ops[..j]);
    }
}

proptest! {



    #[test]
    fn length_prefix_corruption_detected(
        ops in ops_strategy(),
        which in any::<u64>(),
        byte_sel in any::<u64>(),
    ) {
        ensure_linked();
        let n = ops.len() as u64;
        let j = (which % n) as usize;
        let path = unique_path("lenflip");
        write_wal(&path, &ops);

        let ends = end_offsets(&ops);
        let entry_start = if j == 0 { MAGIC_LEN } else { ends[j - 1] };
        let flip_at = entry_start + (byte_sel % 4);

        let mut bytes = std::fs::read(&path).unwrap();
        bytes[flip_at as usize] ^= 0xFF;
        std::fs::write(&path, &bytes).unwrap();

        let got = replay_wal(&path);
        let _ = std::fs::remove_file(&path);
        prop_assert_eq!(got.len(), j, "length corruption in entry {} must stop replay", j);
        prop_assert_eq!(&got[..], &ops[..j]);
    }




    #[test]
    fn zero_crc_is_rejected(
        ops in ops_strategy(),
        which in any::<u64>(),
    ) {
        ensure_linked();
        let n = ops.len() as u64;
        let j = (which % n) as usize;
        let path = unique_path("zerocrc");
        write_wal(&path, &ops);

        let ends = end_offsets(&ops);
        let crc_at = (ends[j] - 4) as usize;
        let mut bytes = std::fs::read(&path).unwrap();

        prop_assume!(bytes[crc_at..crc_at + 4] != [0, 0, 0, 0]);
        bytes[crc_at..crc_at + 4].fill(0);
        std::fs::write(&path, &bytes).unwrap();

        let got = replay_wal(&path);
        let _ = std::fs::remove_file(&path);
        prop_assert_eq!(got.len(), j, "zero CRC in entry {} must be rejected", j);
        prop_assert_eq!(&got[..], &ops[..j]);
    }




    #[test]
    fn torn_tail_then_append_is_reachable(
        ops in ops_strategy(),
        appended in ops_strategy(),
        cut_back in 1u64..=8,
    ) {
        ensure_linked();
        let path = unique_path("tornappend");
        write_wal(&path, &ops);



        let ends = end_offsets(&ops);
        let full = *ends.last().unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(full - cut_back).unwrap();
        drop(f);


        write_wal(&path, &appended);

        let got = replay_wal(&path);
        let _ = std::fs::remove_file(&path);
        let mut want: Vec<(u8, Vec<u8>)> = ops[..ops.len() - 1].to_vec();
        want.extend(appended.iter().cloned());
        prop_assert_eq!(got, want, "appended entries must be reachable after a torn tail");
    }
}
