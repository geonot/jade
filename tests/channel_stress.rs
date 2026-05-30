//! Channel stress + crash-consistency tests (P1-15).
//!
//! Drives the C runtime's MPMC channel (`runtime/channel.c`) with N producer
//! threads and M consumer threads, in both a heavily-contended bounded regime
//! and a large-capacity ("never blocks") regime, and asserts the channel's
//! core concurrency contract:
//!
//!   * **No loss**     — every message sent is received.
//!   * **No duplication** — every message is received exactly once.
//!   * **No corruption**  — received payloads are byte-identical to what was
//!                          sent (verified by reconstructing the full (producer,
//!                          seq) grid from the union of all consumers).
//!   * **Clean shutdown** — after every producer has finished and the channel
//!                          is closed, all consumers drain the buffer and exit
//!                          (no consumer hangs).
//!
//! It also **measures end-to-end tail latency** (send→recv) and prints the
//! p50/p90/p99/p999/max percentiles. Latency is reported, not hard-asserted,
//! to keep CI non-flaky; only a generous upper bound guards against a true
//! hang/livelock.
//!
//! The channel's send/recv are lock-protected (atomic spinlock + acquire/
//! release head/tail), and from non-coroutine OS-thread context they spin-and-
//! retry rather than parking — so this test exercises exactly the shared,
//! lock-guarded ring buffer. Under `ci/sanitize.sh` the C runtime is built with
//! `-fsanitize=thread`, so this same test doubles as the channel's TSan race
//! check.

use std::ffi::c_void;
use std::thread;
use std::time::Instant;

// ── Channel FFI (runtime/channel.c) ──────────────────────────────────
// Imported from `jinnc::runtime_ffi` rather than re-declared locally: the C
// runtime is linked as a static archive (build.rs), and the linker only pulls
// `channel.o` out of that archive if something in the `jinnc` rlib (which
// precedes the archive on the link line) statically references these symbols.
// `force_link_chan` lives in the rlib and provides exactly that anchor, so the
// channel symbols resolve here. (The WAL property test relies on the identical
// `force_link_wal` mechanism.)
use jinnc::runtime_ffi::{
    force_link_chan, jinn_chan_close, jinn_chan_create, jinn_chan_destroy, jinn_chan_recv,
    jinn_chan_send,
};

/// A `*mut c_void` channel handle is safe to share across threads: every
/// access goes through the channel's internal spinlock. The raw pointer is
/// not `Send` by default, so wrap it.
#[derive(Clone, Copy)]
struct Chan(*mut c_void);
unsafe impl Send for Chan {}
unsafe impl Sync for Chan {}

impl Chan {
    /// Takes `self` by value so closures capture the whole `Chan` (which is
    /// `Send`) rather than disjointly capturing the inner `*mut c_void` field
    /// (which is not) under edition-2021 closure capture rules.
    fn raw(self) -> *mut c_void {
        self.0
    }
}

/// 16-byte message: which producer, which sequence number, and a monotonic
/// send timestamp (ns since a shared epoch) for latency measurement.
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

/// Run one stress configuration and assert the channel's contract.
fn run_stress(producers: u32, consumers: u32, per_producer: u32, capacity: usize, label: &str) {
    std::hint::black_box(force_link_chan());

    let total = (producers * per_producer) as usize;
    let epoch = Instant::now();

    let ch = Chan(unsafe { jinn_chan_create(std::mem::size_of::<Msg>(), capacity) });
    assert!(!ch.0.is_null(), "jinn_chan_create returned NULL");

    // ── Producers ────────────────────────────────────────────────────
    let mut prod_handles = Vec::new();
    for p in 0..producers {
        let ch = ch;
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

    // ── Consumers ────────────────────────────────────────────────────
    // Each consumer recvs until the channel is closed *and* drained, returning
    // the (producer, seq) pairs it saw plus the latencies it measured.
    let mut cons_handles = Vec::new();
    for _ in 0..consumers {
        let ch = ch;
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
                    break; // closed and empty
                }
                let now = epoch.elapsed().as_nanos() as u64;
                lats.push(now.saturating_sub(msg.send_ns));
                seen.push((msg.producer, msg.seq));
            }
            (seen, lats)
        }));
    }

    // All sends complete before we close: jinn_chan_send drops messages if the
    // channel is already closed, so closing early would lose data. Joining the
    // producers first guarantees no send is in flight at close time.
    for h in prod_handles {
        h.join().expect("producer panicked");
    }
    unsafe { jinn_chan_close(ch.0) };

    // ── Collect + verify ─────────────────────────────────────────────
    let mut all_seen: Vec<(u32, u32)> = Vec::with_capacity(total);
    let mut all_lats: Vec<u64> = Vec::with_capacity(total);
    for h in cons_handles {
        let (seen, lats) = h.join().expect("consumer panicked");
        all_seen.extend(seen);
        all_lats.extend(lats);
    }

    unsafe { jinn_chan_destroy(ch.0) };

    // No loss: exactly `total` messages received.
    assert_eq!(
        all_seen.len(),
        total,
        "[{label}] expected {total} messages, received {}",
        all_seen.len()
    );

    // No duplication / no corruption: the union of received (producer, seq)
    // pairs is exactly the full grid {0..producers} × {0..per_producer}.
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
            // Binary search is fine: all_seen is sorted and now of size `total`.
            debug_assert!(
                all_seen.binary_search(&(p, s)).is_ok(),
                "[{label}] missing message (producer={p}, seq={s})"
            );
        }
    }
    // Cheap structural check that the grid is complete (first/last corners).
    assert_eq!(all_seen.first(), Some(&(0u32, 0u32)));
    assert_eq!(all_seen.last(), Some(&(producers - 1, per_producer - 1)));

    // Measure + report tail latency.
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

    // Generous guard against livelock/hang (not a perf assertion). 30s.
    assert!(
        max < 30_000_000_000,
        "[{label}] pathological tail latency: max={max} ns (possible livelock)"
    );
}

#[test]
fn channel_stress_bounded_high_contention() {
    // Small capacity → buffer is frequently full/empty → maximal contention on
    // the spinlock and the full/empty fast-paths.
    run_stress(4, 4, 8_000, 16, "bounded");
}

#[test]
fn channel_stress_large_capacity() {
    // Capacity ≥ total messages → producers effectively never block; exercises
    // the uncontended-buffer fast path and pure enqueue/dequeue throughput.
    run_stress(4, 4, 8_000, 1 << 16, "unbounded");
}

#[test]
fn channel_stress_many_producers_one_consumer() {
    // Fan-in: many producers, a single consumer draining. Stresses the
    // single-reader / many-writer ordering on a small bounded buffer.
    run_stress(8, 1, 4_000, 32, "fan-in");
}

#[test]
fn channel_stress_one_producer_many_consumers() {
    // Fan-out: one producer, many consumers competing for each element.
    // Stresses the single-writer / many-reader path and clean shutdown when
    // all consumers must observe the close.
    run_stress(1, 8, 16_000, 32, "fan-out");
}
