use std::ffi::c_void;
use std::thread;
use std::time::Instant;

use jinnc::runtime_ffi::{
    force_link_chan, jinn_chan_close, jinn_chan_create, jinn_chan_destroy, jinn_chan_recv,
    jinn_chan_send,
};

#[derive(Clone, Copy)]
struct Chan(*mut c_void);
unsafe impl Send for Chan {}
unsafe impl Sync for Chan {}

impl Chan {
    fn raw(self) -> *mut c_void {
        self.0
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Msg {
    producer: u32,
    seq: u32,
    send_ns: u64,
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx]
}

fn run_stress(producers: u32, consumers: u32, per_producer: u32, capacity: usize, label: &str) {
    std::hint::black_box(force_link_chan());

    let total = (producers * per_producer) as usize;
    let epoch = Instant::now();

    let ch = Chan(unsafe { jinn_chan_create(std::mem::size_of::<Msg>(), capacity) });
    assert!(!ch.0.is_null(), "jinn_chan_create returned NULL");

    let mut prod_handles = Vec::new();
    for p in 0..producers {
        prod_handles.push(thread::spawn(move || {
            for s in 0..per_producer {
                let msg = Msg {
                    producer: p,
                    seq: s,
                    send_ns: epoch.elapsed().as_nanos() as u64,
                };
                unsafe {
                    jinn_chan_send(ch.raw(), &msg as *const Msg as *const c_void);
                }
            }
        }));
    }

    let mut cons_handles = Vec::new();
    for _ in 0..consumers {
        cons_handles.push(thread::spawn(move || {
            let mut seen: Vec<(u32, u32)> = Vec::new();
            let mut lats: Vec<u64> = Vec::new();
            loop {
                let mut msg = Msg {
                    producer: 0,
                    seq: 0,
                    send_ns: 0,
                };
                let rc = unsafe { jinn_chan_recv(ch.raw(), &mut msg as *mut Msg as *mut c_void) };
                if rc == 0 {
                    break;
                }
                let now = epoch.elapsed().as_nanos() as u64;
                lats.push(now.saturating_sub(msg.send_ns));
                seen.push((msg.producer, msg.seq));
            }
            (seen, lats)
        }));
    }

    for h in prod_handles {
        h.join().expect("producer panicked");
    }
    unsafe { jinn_chan_close(ch.0) };

    let mut all_seen: Vec<(u32, u32)> = Vec::with_capacity(total);
    let mut all_lats: Vec<u64> = Vec::with_capacity(total);
    for h in cons_handles {
        let (seen, lats) = h.join().expect("consumer panicked");
        all_seen.extend(seen);
        all_lats.extend(lats);
    }

    unsafe { jinn_chan_destroy(ch.0) };

    assert_eq!(
        all_seen.len(),
        total,
        "[{label}] expected {total} messages, received {}",
        all_seen.len()
    );

    all_seen.sort_unstable();
    all_seen.dedup();
    assert_eq!(
        all_seen.len(),
        total,
        "[{label}] duplicate or corrupted messages: {} distinct of {total}",
        all_seen.len()
    );
    for p in 0..producers {
        for s in 0..per_producer {
            debug_assert!(
                all_seen.binary_search(&(p, s)).is_ok(),
                "[{label}] missing message (producer={p}, seq={s})"
            );
        }
    }

    assert_eq!(all_seen.first(), Some(&(0u32, 0u32)));
    assert_eq!(all_seen.last(), Some(&(producers - 1, per_producer - 1)));

    all_lats.sort_unstable();
    let p50 = percentile(&all_lats, 0.50);
    let p90 = percentile(&all_lats, 0.90);
    let p99 = percentile(&all_lats, 0.99);
    let p999 = percentile(&all_lats, 0.999);
    let max = *all_lats.last().unwrap_or(&0);
    eprintln!(
        "[{label}] {producers}p×{consumers}c×{per_producer} cap={capacity}  \
         latency ns: p50={p50} p90={p90} p99={p99} p99.9={p999} max={max}"
    );

    assert!(
        max < 30_000_000_000,
        "[{label}] pathological tail latency: max={max} ns (possible livelock)"
    );
}

#[test]
fn channel_stress_bounded_high_contention() {
    run_stress(4, 4, 8_000, 16, "bounded");
}

#[test]
fn channel_stress_large_capacity() {
    run_stress(4, 4, 8_000, 1 << 16, "unbounded");
}

#[test]
fn channel_stress_many_producers_one_consumer() {
    run_stress(8, 1, 4_000, 32, "fan-in");
}

#[test]
fn channel_stress_one_producer_many_consumers() {
    run_stress(1, 8, 16_000, 32, "fan-out");
}
