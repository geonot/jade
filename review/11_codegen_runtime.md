# §11 LLVM codegen

**Files:** `src/codegen/` (29,679 LOC). 25 submodules.

## 11.1 Structural concerns

- **Two parallel codegen paths** (§3): a HIR-direct path in
  `src/codegen/{arith.rs, expr/, stmt/, …}` and a MIR-based path in
  `src/codegen/mir_codegen/`. Both extend `Compiler<'ctx>`. The
  HIR-direct path is the older one and is being retired in favor of
  MIR-based; in the meantime they have **silently drifted** on
  safety semantics.
- **`b!` macro** (`src/codegen/mod.rs:65`) wraps every inkwell call
  result with `.map_err(|e| e.to_string())?`. Fine.
- **`ice!` macro** (`mod.rs:73`) panics with file:line:col when an
  `Option` is `None`. Used liberally; this is where most ICE crashes
  come from. Every `ice!` site is a soundness invariant the upstream
  pass must guarantee.
- **`fn_or_die`** (`mod.rs:92`) panics on missing runtime function
  declarations.

## 11.2 Findings

### F-CG-1 (P0): Integer div/mod-by-zero is UB in the MIR path

Source — `src/codegen/mir_codegen/helpers/values.rs:121-138`:
```rust
mir::BinOp::Div => {
    if result_ty.is_signed() {
        b!(self.bld.build_int_signed_div(li, ri, "sdiv"))
    } else {
        b!(self.bld.build_int_unsigned_div(li, ri, "udiv"))
    }
}
mir::BinOp::Mod => {
    if result_ty.is_signed() {
        b!(self.bld.build_int_signed_rem(li, ri, "srem"))
    } else {
        b!(self.bld.build_int_unsigned_rem(li, ri, "urem"))
    }
}
```
**No zero check.** Meanwhile the HIR-direct path
(`src/codegen/arith.rs:185-191`) correctly dispatches to
`checked_divmod`, which traps with `"division by zero"`.
`checked_divmod` exists, is correct, and is reachable from only the
older code path.

**Reproduce:**
```
$ jinnc - <<'JN' -o /tmp/x  &&  /tmp/x; echo "exit=$?"
*main
    a is 10
    b is 0
    c is a / b
    log(c)
JN
exit=0
                 ← printed: 140737459297944  (uninitialized stack)
```

**Fix:** Make `mir_codegen::helpers::values` call a unified
`emit_checked_divmod` that emits the same trap. Same for `INT_MIN /
-1` (the second UB case for signed division). Delete the unchecked
path.

### F-CG-2 (P0): Vec OOB segfaults in some paths

The `emit_vec_bounds_check` helper exists in `src/codegen/vec/core.rs:543`
and is called from `vec_get_val`, `vec_set_val`, `vec_remove_val`,
some MIR aggregate code paths. The trap path goes through `llvm.trap`
(intrinsic), not through the language's `__jinn_trap("…")` helper —
so the user sees `SIGSEGV` (rc=139) rather than a clean diagnostic.

But probe `p07_oob_read.jn` was an `[i64]` vec literal indexed at `[5]`
which **should** route through the checked path. The fact that it
SIGSEGV's instead of trapping cleanly means either:
(a) the indexed-load was lowered through an MIR aggregate path that
*doesn't* call `emit_vec_bounds_check` (likely — see the
`emit_inst/aggregates.rs` callers — only some patterns), or
(b) the check is emitted but `llvm.trap` is what generates SIGSEGV-equivalent.

In either case, **the user-facing experience is "segfault, no
diagnostic, no line number"**, which is below alpha bar.

**Fix:** Audit every vec/array access lowering in
`mir_codegen/emit_inst/aggregates.rs` (lines 391, 497, 564 already
call the check; find the ones that don't). Replace `llvm.trap` with
`__jinn_trap("array index N out of bounds (len M) at FILE:LINE")` so
the user sees what happened and where.

### F-CG-3 (P0): Generator/yield lowering produces invalid LLVM IR

`p31_generator.jn`:
```
*counts(n as i64)
    for i from 0 to n
        yield i
*main
    for v in counts(5)
        log(v)
```
Output:
```
Function return type does not match operand type of return inst!
  ret i64 0
 ptr
```
LLVM's verifier catches this; a sound compiler would never emit it.
The likely culprit is `src/codegen/coroutines.rs` or the generator
splitting in `mir/lower/`. Either the function's return type is being
declared as `ptr` (for the generator handle) but the body returns the
last `i64`, or the typer is choosing the wrong return type for an
implicitly-returning generator function.

### F-CG-4 (P0): `map(v, $ * 2)` ICE → runtime crash

Discussed in §7 (F-TYPE-3). Codegen side: `helpers/values.rs:96`
panics in probe v1 because the LHS of the multiply is a `PointerValue`
(the slot ptr inside the closure) instead of `IntValue` (the loaded
element). Probe v2 compiles (with the `mir-perceus:` info line) but
runs to rc=16. **The user-visible UX is: idiomatic code crashes; no
diagnostic, no help.**

### F-CG-5 (P1): Auto-widening band-aid for operand-width mismatches

Source — `src/codegen/mir_codegen/helpers/values.rs:91-118`:
> Auto-widen mismatched integer widths to the wider operand.
> Required because MIR currently lets `i32 << i64` reach codegen
> unaltered (e.g. `i * 2` where `i: i32` and `2: i64`).

This is "Linus-bad" code: a comment that documents the workaround
without addressing the root cause. The root cause is typer/MIR not
producing well-typed operands; see F-TYPE-1, F-MIR-1. Once those are
fixed, this block can be deleted.

### F-CG-6 (P2): Codegen is 35% of compiler LOC; should be ≤ 20%

The center of gravity is in the wrong place. Most of the size comes
from:
- Per-construct specialization (vec, map, channel, store, actor,
  coroutine each have their own subdirectory of emit logic).
- Defensive type recovery (auto-widen, pointer-arith fallback,
  `clone_value` deep-copy logic).
- Two parallel paths (HIR-direct + MIR-based).

§24 contains a refactor plan.

### F-CG-7 (P2): `mir-perceus:` info line leaks (F-MIR-3 also)

Already noted.

### F-CG-8 (P3): `unsafe { build_gep }` is fine but unaudited

171 `unsafe` blocks in the compiler crate. The vast majority appear
to be inkwell's `unsafe` GEP builder (`bld.build_gep(...)` is
unsafe because GEP wants caller-justified element/pointer typing).
Worth a once-over for any that aren't justified.

## 11.3 What's correct

- Bounds check helper is in place and works for the access patterns
  that route through it.
- Drop/Clone insertion is methodical.
- The arithmetic happy path (probe `p02_overflow.jn`,
  `p03_floats.jn`, `p42_cast.jn`) produces correct results.
- Tail calls are recognised and emitted as `tail` / become
  `musttail` candidates — probe `p41_tco.jn` (10 M-deep) returns
  10000000 correctly.
- 16/16 sample apps and 33/33 benchmarks compile to working
  binaries.

## 11.4 Verdict

**Not alpha-ready** due to F-CG-1 (div-by-zero UB), F-CG-2 (OOB
SIGSEGV without diagnostic), F-CG-3 (invalid IR for generators),
F-CG-4 (closure-over-vec ICE/runtime crash). All four are
mechanically fixable in days, not weeks.

---

# §12 C runtime review

**Files:** `runtime/jinn_rt.h` + 31 `.c` / `.S` files (≈ 6.3 k LOC).
Built into `libjinn_rt.a`.

## 12.1 Inventory

| File | LOC range | Role |
| ---- | --------- | ---- |
| `sched.c` | medium | M:N work-stealing scheduler |
| `coro.c` | medium | Stackful coroutine lifecycle |
| `context_x86_64.S` / `context_aarch64.S` | small | Context switch in assembly |
| `channel.c` | medium | Typed channels with send/recv waitqs + spinlock |
| `actor.c` | medium | Actor mailbox + dispatch |
| `select.c` | small | Multi-channel `select` |
| `sup.c` | small | Supervisor strategies |
| `timer.c` | small | Timer wheel for sleep |
| `deque.c` | small | Lock-free work-stealing deque |
| `wal.c` | medium | Write-ahead log |
| `kv.c`, `vec.c`, `vector.c`, `column.c`, `bloom.c`, `fts.c`, `index.c` | medium-large | Persistent / indexed stores |
| `sqlite.c` | small | Thin wrapper around system sqlite |
| `crypto.c` | small | OpenSSL or fallback shims |
| `net.c`, `tls.c`, `fs.c`, `process.c`, `terminal.c` | small-med | Syscall wrappers |
| `regex_helper.c` | small | Regex engine support |
| `random.c`, `util.c`, `event.c`, `migrate.c`, `version.c` | small | misc |

**Quality signal:** Zero `TODO` / `FIXME` / `HACK` / `XXX` comments
anywhere. Either pristine or scrubbed. Either way, debt is unlabelled.

## 12.2 Findings

### F-RT-1 (P0 for production, P1 for alpha): No sanitizer runs in CI

A runtime this concurrent (stackful coroutines, lock-free deque,
channel spinlocks, atomic store ordering) absolutely needs ASan/TSan
runs in CI before any release tag. I see no evidence either has been
run.

### F-RT-2 (P1): 64 KB fixed coroutine stack with 4 KB guard

```c
#define JINN_STACK_SIZE  (64 * 1024)
#define JINN_GUARD_SIZE  4096
```
64 KB is tight for any code that uses deep recursion through actor
handlers (Jinn programs *will* do this — see `apps/raft_cluster/`).
The guard page protects against silent stack-stomp into adjacent
data, but it converts stack overflow into SIGSEGV without a
diagnostic. **Recommendation:** install a SIGSEGV handler that
checks `siginfo->si_addr` against the guard range and prints
"coroutine stack overflow at FILE:LINE" before re-raising.
Stack-growable coroutines are nice-to-have for beta.

### F-RT-3 (P1): Spinlocks in channels — fairness?

`runtime/channel.c` uses spinlocks for waitq protection. Under high
contention this risks priority inversion in the M:N runtime. Worth
profiling under stress before alpha.

### F-RT-4 (P2): No documentation of the runtime ABI

Compiler emits calls into `jinn_*` runtime functions; the contract is
implicit. A single `runtime/ABI.md` would prevent codegen↔runtime
drift. The `fn_or_die` ICE site exists *because* this contract is
informal.

### F-RT-5 (P2): Setjmp fallback for unknown architectures

```c
#else
#include <setjmp.h>
typedef struct { jmp_buf env; } jinn_context_t;
#endif
```
This fallback is dead code in practice (x86_64 and aarch64 dominate)
but `setjmp/longjmp` cannot actually substitute for a context-switch
between coroutines — it only supports one-direction non-local jumps
within a single stack. Anyone who builds Jinn on a non-supported arch
will silently get a broken runtime. **Better to refuse to build with
a clear error** until that arch gets real assembly.

### F-RT-6 (P3): `_Thread_local jinn_coro_t *tl_gen_coro;` global

Generator/coroutine state in thread-local globals is OK but couples
generators to a single thread. Document this; it's an actor-runtime
invariant.

## 12.3 What's correct

- The header is clean, well-commented, and the public ABI is
  obviously deliberate (everything prefixed `jinn_*`).
- Arch-specific context switch in actual assembly (not setjmp) on
  the two architectures that matter.
- Stackful-coroutine model is the right primitive for the language's
  async story (cleaner than stackless with explicit `Future` types).
- Opaque persistent-store types (`JinnBloom`, `JinnKV`, etc.) keep
  the public ABI small.

## 12.4 Verdict

**Solid foundation, not alpha-ready without sanitizer runs and a
proper stack-overflow diagnostic** (F-RT-1, F-RT-2).
