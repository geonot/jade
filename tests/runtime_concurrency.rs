//! Conformance tests for the Tier 2 runtime-concurrency fixes
//! (remediation-2026-07.md tasks 8-10..8-14). Each test compiles and runs a
//! real program through `jinnc`, asserting exact output within a timeout —
//! the failure modes here are hangs, SIGSEGVs, and allocator aborts.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

struct Compiled {
    dir: tempfile::TempDir,
}

fn compile(src: &str) -> Compiled {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("prog.jn"), src).unwrap();
    let out = Command::new(jinnc())
        .arg("prog.jn")
        .arg("-o")
        .arg(dir.path().join("prog.bin"))
        .current_dir(dir.path())
        .output()
        .expect("invoke jinnc");
    assert!(
        out.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Compiled { dir }
}

impl Compiled {
    /// Run with a hard timeout; a hang is a failure, not a stuck suite.
    fn run_within(&self, secs: u64) -> Output {
        let mut child = Command::new(self.dir.path().join("prog.bin"))
            .current_dir(self.dir.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn program");
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            match child.try_wait().expect("try_wait") {
                Some(_) => return child.wait_with_output().expect("collect output"),
                None if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let out = child.wait_with_output().expect("collect output");
                    panic!(
                        "program hung past {secs}s: stdout so far={:?} stderr={:?}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    }
}

fn sorted_lines(out: &[u8]) -> Vec<i64> {
    let mut v: Vec<i64> = String::from_utf8_lossy(out)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    v.sort();
    v
}

/// Task 8-12 — cancelling a scope after some children have already
/// completed used to iterate freed coroutines: `s->children[]` was never
/// pruned on exit, while the scheduler `jinn_coro_destroy`ed the child, so
/// `jinn_scope_cancel` read `->cancelled`/`->wait_chan` of freed memory
/// and could enqueue a freed coroutine (heap-use-after-free under ASan,
/// 3/3 before the fix). Four quick children complete and are destroyed
/// long before the fifth errors and triggers cancellation.
#[test]
fn scope_cancel_after_children_completed() {
    let src = "\
err Boom
    Bang

*quick(id as i64)
    log(id)

*slow_fail(n as i64) returns i64 ! Boom
    t is 0
    for i in 0 to n
        t is (t * 3 + i) % 1000003
    if t >= 0
        err Bang
    t

*work() returns i64 ! Boom
    together
        dispatch
            quick(1)
        dispatch
            quick(2)
        dispatch
            quick(3)
        dispatch
            quick(4)
        dispatch
            x is slow_fail(30000000) ? $ !! err
            log(x)
    ? log(7)
    !! log(0 - 1)
    0

*main
    match work()
        Ok(v) ? log(v)
        Err(e) ? log(0 - 2)
    log(99)
";
    let c = compile(src);
    for round in 0..3 {
        let out = c.run_within(15);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            sorted_lines(&out.stdout),
            vec![-1, 0, 1, 2, 3, 4, 99],
            "round {round}"
        );
    }
}

/// Task 8-12 — the child registry used to be a fixed 64-slot array;
/// children past 64 were counted but silently not registered, so
/// cancellation missed them. The registry now grows: 100 children all
/// register, run, and join.
#[test]
fn scope_handles_more_than_64_children() {
    let dispatches: String = (1..=100)
        .map(|i| format!("        dispatch\n            task({i})\n"))
        .collect();
    let src =
        format!("*task(id as i64)\n    log(id)\n\n*main\n    together\n{dispatches}    log(999)\n");
    let c = compile(&src);
    let out = c.run_within(15);
    assert!(out.status.success(), "{:?}", out.status);
    let mut expected: Vec<i64> = (1..=100).collect();
    expected.push(999);
    assert_eq!(sorted_lines(&out.stdout), expected);
}

/// Task 8-14 — one worker queueing past the deque's 1024-slot initial
/// buffer forces grows while other workers steal. Before the fix, grow
/// freed the old buffer under live thieves and published buffer/capacity
/// as a torn pair. 5001 tasks force at least two grows; every task must
/// run exactly once.
#[test]
fn deque_grow_under_stealing_runs_every_task_once() {
    let src = "\
*work(id as i64)
    log(id)

*main
    together
        for i in 0 to 5000
            dispatch
                work(i)
    log(9999)
";
    let c = compile(src);
    let out = c.run_within(30);
    assert!(out.status.success(), "{:?}", out.status);
    let mut expected: Vec<i64> = (0..5000).collect();
    expected.push(9999);
    assert_eq!(sorted_lines(&out.stdout), expected);
}

/// Task 8-14 — the standalone C harness (tests/deque_stress.c) drives the
/// deque from raw pthreads, which is the only way to get a meaningful
/// TSan signal (the fiber-swapping scheduler has no TSan annotations).
/// Old deque: TSan data-race and ASan heap-use-after-free, 5/5. Skips a
/// sanitizer leg gracefully when the toolchain lacks it.
#[test]
fn deque_stress_harness_is_sanitizer_clean() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = tempfile::tempdir().unwrap();
    for san in ["thread", "address"] {
        let bin = dir.path().join(format!("deque_{san}"));
        let cc = Command::new("cc")
            .args([
                "-O1",
                "-g",
                "-fno-omit-frame-pointer",
                &format!("-fsanitize={san}"),
                "-Iruntime",
                "tests/deque_stress.c",
                "runtime/deque.c",
                "-lpthread",
                "-o",
            ])
            .arg(&bin)
            .current_dir(&root)
            .output()
            .expect("invoke cc");
        if !cc.status.success() {
            eprintln!(
                "skipping -fsanitize={san}: {}",
                String::from_utf8_lossy(&cc.stderr)
            );
            continue;
        }
        for round in 0..3 {
            let out = Command::new(&bin).output().expect("run harness");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(
                out.status.success()
                    && !stderr.contains("ThreadSanitizer")
                    && !stderr.contains("AddressSanitizer"),
                "-fsanitize={san} round {round}: status={:?}\n{stderr}",
                out.status
            );
        }
    }
}

/// Task 8-10 — sustained scope-join + actor-join + channel park/wake churn.
/// Every park site now hands its guard lock to the scheduler so no waker
/// can swap into a half-saved context, and an exited actor's mailbox is
/// retired rather than freed, so `stop e; join e` on a fast-draining actor
/// no longer writes the join slot of freed memory (heap-use-after-free,
/// found by this very stress under ASan; clean 3/3 after). 200 rounds of
/// spawn/bump/stop/join inside nested scopes.
#[test]
fn park_handoff_actor_join_churn() {
    let src = "\
actor Echo
    total as i64

    @bump n as i64
        total is total + n

*main
    for round in 0 to 200
        ch1 is channel of i64(1)
        together
            dispatch
                send ch1, 1
            dispatch
                v is receive ch1
                log(v)
            dispatch
                e is spawn Echo
                e.bump(1)
                stop e
                join e
        close ch1
    log(777)
";
    let c = compile(src);
    for round in 0..3 {
        let out = c.run_within(60);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let mut expected: Vec<i64> = vec![1; 200];
        expected.push(777);
        assert_eq!(sorted_lines(&out.stdout), expected, "round {round}");
    }
}

/// Task 8-13 — `tl_gen_coro` was set on every `jinn_gen_resume` but cleared
/// only on the trampoline's first entry, so a worker that resumed a
/// generator more than once kept a stale value forever, and the next fresh
/// coroutine whose first run landed there executed the *generator's* entry
/// instead of its own (SIGSEGV, 3/3 before the fix). Round 1 poisons every
/// worker (more advancing tasks than workers, several resumes each); round
/// 2's fresh coroutines must still run their own bodies.
#[test]
fn generator_resume_does_not_poison_worker_tls() {
    let advance: String = (1..=8)
        .map(|i| format!("        dispatch\n            advance({i}000)\n"))
        .collect();
    let fresh: String = (11..=18)
        .map(|i| format!("        dispatch\n            fresh({i})\n"))
        .collect();
    let src = format!(
        "*counter()\n    n is 0\n    loop\n        yield n\n        n is n + 1\n\n\
         *advance(id as i64)\n    g is counter()\n    a is g.next()\n    b is g.next()\n    c is g.next()\n    log(a + b + c + id)\n\n\
         *fresh(id as i64)\n    log(id)\n\n\
         *main\n    together\n{advance}    together\n{fresh}    log(999)\n"
    );
    let c = compile(&src);
    for round in 0..3 {
        // Generous timeout: the whole suite runs in parallel and each jinn
        // program spawns a full worker pool, so wall-clock under load is
        // many times the ~0.15s standalone time.
        let out = c.run_within(60);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let mut expected: Vec<i64> = (11..=18).collect();
        expected.extend([999]);
        expected.extend((1..=8).map(|i| i * 1000 + 3));
        expected.sort();
        assert_eq!(
            sorted_lines(&out.stdout),
            expected,
            "round {round}: wrong output"
        );
    }
}

/// Task 8-11 — a selector must sit on EVERY case's wait queue. The old
/// single intrusive `next` pointer enqueued on one channel per attempt
/// (round-robin), so a selector parked on ch1 was never woken by traffic
/// on ch2 — a hang. Run the wake through each channel in turn. The delay
/// guarantees the selector parks before the send, and `run_within` turns
/// the old hang into a test failure. Also pins the 256-retry "possible
/// deadlock" masquerade: no default arm exists, so the select must block
/// until genuinely ready and stderr must stay silent.
#[test]
fn select_parked_is_woken_by_either_channel() {
    let src = "\
*delay(n as i64) returns i64
    t is 0
    for i in 0 to n
        t is (t * 3 + i) % 1000003
    t

*sel_two(target as i64)
    ch1 is channel of i64(4)
    ch2 is channel of i64(4)
    together
        dispatch
            select
                receive ch1 as v
                    log(v)
                receive ch2 as v
                    log(v)
        dispatch
            x is delay(3000000)
            if x >= 0
                if target equals 1
                    send ch1, 1
                else
                    send ch2, 2
    close ch1
    close ch2

*main
    sel_two(1)
    sel_two(2)
    log(99)
";
    let c = compile(src);
    for round in 0..3 {
        let out = c.run_within(60);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("deadlock"),
            "round {round}: retry exhaustion resurfaced: {stderr}"
        );
        assert_eq!(sorted_lines(&out.stdout), vec![1, 2, 99], "round {round}");
    }
}

/// Task 8-11 — close was never checked: a select-receive on a closed empty
/// channel parked forever because close only woke waiters present at close
/// time, and the readiness scan had no `closed` test. Cover both windows:
/// channel closed before the select (scan must see it) and closed while
/// the selector is parked (the close wake must fire the case). A fired
/// close-receive yields the zero value, exactly like `jinn_chan_recv`.
#[test]
fn select_observes_close_before_and_during_park() {
    let src = "\
*delay(n as i64) returns i64
    t is 0
    for i in 0 to n
        t is (t * 3 + i) % 1000003
    t

*pre_closed()
    ch is channel of i64(4)
    close ch
    select
        receive ch as v
            log(v + 10)

*closed_while_parked()
    ch1 is channel of i64(4)
    ch2 is channel of i64(4)
    together
        dispatch
            select
                receive ch1 as v
                    log(v + 20)
                receive ch2 as v
                    log(v + 30)
        dispatch
            x is delay(3000000)
            if x >= 0
                close ch2
    close ch1

*main
    pre_closed()
    closed_while_parked()
    log(99)
";
    let c = compile(src);
    for round in 0..3 {
        let out = c.run_within(60);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(sorted_lines(&out.stdout), vec![10, 30, 99], "round {round}");
    }
}

/// Task 8-11 — the old `int poll_order[16]` with `limit = min(n, 16)`
/// silently never polled cases 17+. A 32-case select whose only ready
/// channel is case index 20 must fire it, not hang.
#[test]
fn select_with_32_cases_fires_beyond_old_cap() {
    let decls: String = (0..32)
        .map(|i| format!("    ch{i} is channel of i64(4)\n"))
        .collect();
    let arms: String = (0..32)
        .map(|i| format!("                receive ch{i} as v\n                    log(v + {i})\n"))
        .collect();
    let src = format!(
        "*main\n{decls}    together\n        dispatch\n            select\n{arms}        dispatch\n            send ch20, 100\n    log(999)\n"
    );
    let c = compile(&src);
    for round in 0..3 {
        let out = c.run_within(60);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(sorted_lines(&out.stdout), vec![120, 999], "round {round}");
    }
}

/// Task 8-11 — exactly-once delivery under contention: 4 selectors race
/// over two channels for 200 items total (2 producers x 100). Every item
/// must be received exactly once — a claim-CAS bug would double-fire or
/// drop one — and the counts work out only if every selector completes
/// all 50 of its selects, so a lost wake is a hang caught by the timeout.
#[test]
fn select_contention_delivers_every_item_exactly_once() {
    let consumers: String = (0..4)
        .map(|k| {
            format!(
                "        dispatch\n            k{k} is 0\n            while k{k} < 50\n                select\n                    receive ch1 as v\n                        log(v)\n                    receive ch2 as v\n                        log(v)\n                k{k} is k{k} + 1\n"
            )
        })
        .collect();
    let src = format!(
        "*main\n    ch1 is channel of i64(128)\n    ch2 is channel of i64(128)\n    together\n        dispatch\n            i is 0\n            while i < 100\n                send ch1, 1 + i\n                i is i + 1\n        dispatch\n            j is 0\n            while j < 100\n                send ch2, 101 + j\n                j is j + 1\n{consumers}    log(999)\n"
    );
    let c = compile(&src);
    for round in 0..3 {
        let out = c.run_within(60);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let mut expected: Vec<i64> = (1..=200).collect();
        expected.push(999);
        assert_eq!(sorted_lines(&out.stdout), expected, "round {round}");
    }
}

/// Task 8-11 — fairness: with two always-ready channels, the shuffled poll
/// order must let both cases fire. 200 draws from channels pre-filled with
/// 200 items each; a starved case (P = 2^-200 under a fair shuffle) means
/// the shuffle or the off-worker rng seeding regressed.
#[test]
fn select_fairness_both_ready_cases_fire() {
    let src = "\
*main
    ch1 is channel of i64(256)
    ch2 is channel of i64(256)
    i is 0
    while i < 200
        send ch1, 1
        send ch2, 2
        i is i + 1
    a is 0
    b is 0
    j is 0
    while j < 200
        select
            receive ch1 as v
                a is a + v
            receive ch2 as v
                b is b + v
        j is j + 1
    log(a)
    log(b)
";
    let c = compile(src);
    for round in 0..3 {
        let out = c.run_within(60);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let lines: Vec<i64> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect();
        assert_eq!(lines.len(), 2, "round {round}: {:?}", lines);
        let (n1, n2) = (lines[0], lines[1] / 2);
        assert_eq!(n1 + n2, 200, "round {round}: draw count off");
        assert!(
            n1 >= 20 && n2 >= 20,
            "round {round}: starved case (ch1={n1}, ch2={n2})"
        );
    }
}

/// Tasks 8-10 + 8-11 — the combined park stress 8-10's DoD asked for,
/// deferred until select could sit on every case queue: each round runs a
/// select (racing a ch1 send against a ch2 close), actor spawn/stop/join,
/// and scope join concurrently, so all park sites and the multi-lock
/// select handoff churn against each other. Either select case may win a
/// round: 1 (ch1's value) or 10 (ch2 closed, zero value + 10). Also run
/// under ASan by `scripts/` sweeps — see task notes.
#[test]
fn select_scope_actor_combined_churn() {
    let src = "\
actor Echo
    total as i64

    @bump n as i64
        total is total + n

*delay(n as i64) returns i64
    t is 0
    for i in 0 to n
        t is (t * 3 + i) % 1000003
    t

*main
    for round in 0 to 200
        ch1 is channel of i64(1)
        ch2 is channel of i64(1)
        together
            dispatch
                send ch1, 1
            dispatch
                select
                    receive ch1 as v
                        log(v)
                    receive ch2 as v
                        log(v + 10)
            dispatch
                x is delay(2000)
                if x >= 0
                    close ch2
            dispatch
                e is spawn Echo
                e.bump(1)
                stop e
                join e
        close ch1
    log(777)
";
    let c = compile(src);
    for round in 0..3 {
        let out = c.run_within(60);
        assert!(
            out.status.success(),
            "round {round}: {:?} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        let lines = sorted_lines(&out.stdout);
        assert_eq!(lines.len(), 201, "round {round}: {lines:?}");
        assert_eq!(*lines.last().unwrap(), 777, "round {round}");
        assert!(
            lines[..200].iter().all(|v| *v == 1 || *v == 10),
            "round {round}: unexpected select result in {lines:?}"
        );
    }
}
