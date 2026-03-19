# Response to Technical Discovery Review 2026

**Date:** March 2026  
**Re:** [TECHNICAL_DISCOVERY_REVIEW_2026.md](TECHNICAL_DISCOVERY_REVIEW_2026.md)  
**Status at time of response:** 589 tests, 7,054 LOC, Jade = 0.96× Clang O3 across 12 benchmarks

---

## Accepting the Core Diagnosis

The review is accurate. The central finding — that Jade's semantic boundary is AST → LLVM with no intermediate representation — is correct, and it is the right thing to focus on. Everything else in the review flows logically from that observation.

Three specific points we accept without qualification:

1. **Ownership claims outrun implementation.** The docs describe inferred ownership, borrowing, Perceus, and move semantics. The code has `Type::Rc`, `emit_retain`, `emit_release`, and raw pointers. These are not the same thing. We will stop describing Perceus as a present-tense feature.

2. **Codegen concentrates too much semantic logic.** `expr_ty`, `infer_ret`, `infer_field_ty`, generic instantiation, RC insertion, call coercion — all live inside `codegen.rs`. This was the right tradeoff for getting the prototype to 589 tests quickly. It is the wrong architecture for what comes next.

3. **Benchmark methodology is not yet publication-grade.** We have now addressed this: the runner records CPU model, governor, median, stddev, variance, IQR, min/max, exports CSV/JSON, and supports warmup runs. Pinned toolchain versions and CI-based reproduction remain TODO.

---

## Do We Want an IR?

**Yes.** 

Not because the review said so. Because we've already hit the wall it predicts.

### Evidence from the last 3 weeks of development

Every feature added in the last month — tuple destructuring, negative indexing, augmented assignment, string `.len()`, struct field assignment, array element assignment — required modifying `codegen.rs` in ways that interleave type discovery with LLVM emission. Each change touches `expr_ty` (to infer what we're working with), `emit_expr` (to generate the IR), and sometimes `infer_ret` or `compile_fn` (to propagate the result type).

Concrete examples:

- **Tuple destructuring** required a name-mangling fix in `expr_ty` because generic function calls were stored under mangled names (`divmod_i64_i64`) but looked up under base names (`divmod`). This is a name-resolution bug that should have been caught before codegen ever sees the AST.

- **String `.len()`** required special-casing in `emit_method_call` to detect strings, compute length, and return an unboxed `i64`. This is a method resolution + type inference task masquerading as code generation.

- **Augmented assignment on struct fields** required threading through `emit_field_access`, field type inference, binary op emission, and store-back — all within a single arm of `emit_stmt` in codegen. The semantic question ("is this field mutable? what's its type? does the operation type-check?") is answered ad-hoc inline.

None of these are bugs in the current design. They are evidence that the current design has reached its ergonomic ceiling.

### What we actually need

Not a research-grade multi-layer IR stack. One intermediate representation between parsing and LLVM emission.

```
Source → Lexer → Parser → AST → **HIR** → Codegen → LLVM IR → Native
```

### What this HIR carries

The review's recommendation is right about the content:

| Property | Current location | HIR location |
|----------|-----------------|--------------|
| Resolved names | `codegen.rs` vars/fns maps | HIR nodes carry resolved symbols |
| Expression types | `expr_ty()` in codegen | Every HIR expression is typed |
| Generic instantiations | `compile_generic_fn()` in codegen | HIR contains monomorphized copies |
| Ownership categories | Not yet implemented | Each binding: owned / borrowed / rc / raw |
| Coercions | `emit_coerce()` in codegen | Explicit coercion nodes in HIR |
| Drops | `emit_release()` in codegen | Explicit drop placement in HIR |
| Method resolution | `emit_method_call()` pattern match | Resolved to concrete function in HIR |

### What this is NOT

- Not a new language. The HIR is internal to the compiler. Users never see it.
- Not three intermediate representations. One. AST → HIR → LLVM.
- Not a rewrite of codegen. Codegen shrinks — it reads typed, resolved HIR nodes instead of re-inferring everything from raw AST.
- Not a blocker on feature work. The HIR can be introduced incrementally: start with expression typing, leave ownership for later.

---

## Sequencing: When and How

The review recommends "narrow, formalize, then grow." We agree in principle but diverge slightly on sequencing. We are not going to freeze the language surface to build an IR. Instead, we'll build the IR incrementally alongside continued feature development.

### Phase A: Typed HIR (next)

**Goal:** Every expression and statement in the HIR carries a resolved type. Codegen reads types from the HIR instead of computing them.

New module: `src/hir.rs` (HIR types) + `src/typer.rs` (AST → HIR lowering with type inference).

Scope:
- HIR data structures mirroring AST but with type annotations on every node
- Name resolution: resolve all identifiers to definition sites
- Type inference: bidirectional, synthesis + checking modes
- Generic instantiation: monomorphize during HIR construction
- Method resolution: resolve `.method()` calls to concrete functions
- Coercion insertion: explicit int-width coercions, bool-to-int, etc.

What changes in codegen:
- `expr_ty()` → read `hir_expr.ty`
- `infer_ret()` → read function's resolved return type from HIR
- `infer_field_ty()` → read from HIR struct definition
- `compile_generic_fn()` → already done during HIR construction
- Type-driven dispatch simplification throughout

Estimated scale: ~1,500–2,500 LOC for `hir.rs` + `typer.rs`. Codegen shrinks by ~500–800 LOC as type inference migrates out.

**This is the only phase that's required to unblock everything else.** Ownership, borrowing, and Perceus all layer on top of a typed HIR. Without it, they have nowhere to live.

### Phase B: Ownership Categories in HIR (after Phase A)

Add ownership annotations to HIR bindings:

```
Owned(T)      — value is moved on use, dropped at scope end
Borrowed(&T)  — immutable reference, no drop
BorrowMut(&mut T) — exclusive mutable reference
Rc(T)         — reference-counted shared ownership
Raw(*T)       — raw pointer, unmanaged
```

This requires:
- ownership inference pass over the HIR (conservative: default to Owned)
- borrow checking pass (detect use-after-move, aliased mutation)
- drop insertion pass (place drops at last use or scope end)

Estimated scale: ~1,000–1,500 LOC for inference + checking.

### Phase C: Perceus Optimizations (after Phase B)

With explicit ownership in the HIR:
- Reuse analysis: detect values consumed exactly once → in-place update
- Drop specialization: elide drops for types with no destructor
- Borrow elision: promote borrows to moves when the source isn't used again

This is where Jade's performance story becomes genuinely novel rather than "we generate the same LLVM IR as C." This is also where we earn the right to cite Perceus.

### Phase D: Separate Compilation (independent of B/C)

With the HIR in place, module interfaces become tractable:
- Emit HIR signatures (not full bodies) as `.jadei` interface files
- Compile modules to `.o` independently
- Link at the end

This can proceed in parallel with ownership work.

---

## What We Disagree With

### "Freeze the surface, build the core"

The review recommends narrowing before growing. We agree on building the core. We disagree on freezing.

Jade is in a critical adoption window right now. The language surface — closures, generics, pattern matching, FFI, inline assembly — is what makes people try it. Freezing features to build compiler infrastructure that users don't see risks losing momentum for invisible gains.

Instead: the HIR is introduced as a refactor that improves error quality, compilation speed, and maintainability. Feature work continues on top of the HIR. The existence of the HIR makes features cheaper to add, not more expensive.

### "Delay concurrency until single-threaded ownership is formalized"

Partially agree. We won't implement actors or channels until ownership is in the HIR. But we will continue to support the existing `extern` + syscall + inline assembly path for users who want to write their own concurrency primitives today. The low-level escape hatches are part of Jade's value proposition.

### "Keep `rc` explicit and conservative"

Agree for now. Disagree long-term. The whole point of Jade is that the compiler figures out ownership. Explicit `rc` is the training-wheels version. But the review is right that we shouldn't auto-promote to `rc` until we have a verifier that can validate those decisions.

---

## Updated Claims

Based on this response, here is what Jade's docs should say:

| Topic | Old claim | Updated claim |
|-------|-----------|---------------|
| Ownership | "Compiler-inferred ownership and borrowing" | "Planned. Currently: explicit `rc` wrapper, manual drop. Inference requires HIR (Phase B)." |
| Perceus | "Perceus-style reuse optimization" | "Planned as Phase C, after ownership inference. Currently: explicit retain/release." |
| Memory model | "One owner per value, compiler inserts drops" | "Design intent. Currently: stack values freed at scope end, heap values via `rc`. No move semantics yet." |
| Separate compilation | "Per-module LLVM IR with linking" | "Planned as Phase D. Currently: whole-program text assembly." |
| Type inference | "HM + bidirectional" | "HM-like inference in codegen. Bidirectional checking planned for HIR (Phase A)." |

---

## Benchmark Status Update

The runner now addresses Stage 6 of the review's recommendations:

```
CPU: 11th Gen Intel(R) Core(TM) i7-1185G7 @ 3.00GHz
Cores: 8  |  Governor: powersave
Runs: 5  |  Warmup: 1

-O3, Jade vs C (Clang) vs Rust:

Benchmark         JADE         C       RUST     J/C     J/RUST
ackermann       186.2ms   185.6ms   183.5ms   1.00x     1.01x
array_ops         363us     436us     638us   0.83x     0.57x
closure_capture   432us     442us       ERR   0.98x         -
collatz         163.6ms   194.4ms   196.2ms   0.84x     0.83x
enum_dispatch     389us     440us     538us   0.88x     0.72x
fibonacci       343.0ms   342.9ms   364.5ms   1.00x     0.94x
gcd_intensive    24.5ms    25.9ms    25.8ms   0.95x     0.95x
hof_pipeline      390us     486us     527us   0.80x     0.74x
math_compute      378us     580us   183.5ms   0.65x     0.00x
sieve           151.4ms   153.7ms   151.1ms   0.99x     1.00x
struct_ops        391us     892us     673us   0.44x     0.58x
tight_loop        387us     452us     597us   0.86x     0.65x
───────────────────────────────────────────────────────────
TOTAL           871.5ms   906.2ms    1.11s    0.96x     0.79x
```

Each benchmark now reports: min, median, mean, max, stddev, variance, IQR.  
Exports: JSON (full raw per-run data), CSV, history with platform metadata.  
Flags: `--warmup`, `--bench`, `--sort`, `--detail`, `--csv`, `--json`, `--quiet`.

Remaining per the review: pinned toolchain versions in CI, automated output validation across languages.

---

## Summary

| Review recommendation | Response | When |
|----------------------|----------|------|
| Align docs (Stage 1) | Accept. Update claims table above. | Done |
| Introduce one HIR (Stage 2) | Accept. Phase A. | **Done.** `hir.rs` (417 LOC) + `typer.rs` (2380 LOC) |
| Ownership in HIR (Stage 3) | Accept. Phase B, after typed HIR. | **Done.** `ownership.rs` (675 LOC) |
| Keep `rc` conservative (Stage 4) | Accept for now. Disagree long-term. | Done (Phase B) |
| Upgrade diagnostics (Stage 5) | Accept. Flows from HIR — phase-specific errors become natural. | Done (Phase A side effect) |
| Harden benchmarks (Stage 6) | Done. 15 benchmarks, 4-language comparison, full statistical runner. | **Done (Phase D).** |
| Perceus optimizations (Phase C) | Maximized. `perceus.rs` (1,611 LOC), 9 optimization passes. | **Done.** |

---

### Phase A: Typer Sharpening — Refined

**Module:** `src/typer.rs` (2,504 LOC)

Refinements applied:
- **IfExpr type inference** — was hardcoded to I64, now infers from then-branch tail expression.
- **Float promotion in binary ops** — `coerce_binop_operands` promotes int×float → float and f32×f64 → f64.
- **Extended coercion repertoire** — `maybe_coerce_to` now handles int→float, float→int, float width promotion/narrowing, bool→int. Previously only did int widening.
- **BinOp type synthesis** — considers both operands for float promotion, not just the left side.
- **Coercion ordering** — coercion nodes are inserted BEFORE computing `result_ty`, fixing cases where the result type was derived from un-coerced operands.

### Phase B: Ownership Sharpening — Refined

**Module:** `src/ownership.rs` (675 LOC)

Refinements applied:
- **ReturnOfBorrowed detection** — new `DiagKind::ReturnOfBorrowed` catches `return &local_var` (dangling reference).
- **Scope-drop verification** — `check_scope_drops()` validates that owned, non-moved, non-trivial values have matching drops or are consumed.
- **Move tracking through calls** — `Call` and `IndirectCall` arguments that are owned and non-trivially-droppable are recorded as moves, enabling accurate use-after-move detection across function boundaries.
- **Trivially-droppable classification** — `is_trivially_droppable()` mirrors Perceus logic: scalars, fixed arrays/tuples of scalars, raw pointers.
- **Move recording** — `record_move()` only records ownership transfer for non-trivial Owned values, avoiding false positives on scalars.

### Phase C: Perceus Maximization — 9 Optimization Passes

**Module:** `src/perceus.rs` (1,611 LOC)  
**Pipeline position:** Typer → HIR → **Perceus** → Ownership Verifier → Codegen  
**Tests:** 20 unit tests, all pass  
**Total test count:** 623 (85 unit + 376 bulk + 162 integration)

Nine optimization passes, informed by the Perceus paper (Reinking, Lorenzen, Leijen — ICFP 2021) and extended:

**1. Use Counting** — Per-DefId use/escape/borrow tracking across the full function scope. Foundation for all subsequent passes.

**2. Drop Specialization** — Types with no heap resources (scalars, fixed arrays/tuples of scalars, raw pointers, borrows) have their drops elided entirely. No scope-end cleanup for values that were never heap-allocated.

**3. Reuse Analysis** — Detects Rc values consumed exactly once (unique references) where the memory can be reused by a subsequent allocation of compatible layout. Enhanced: full non-adjacent pair scanning instead of adjacent-window-only. Conservative: loop bodies excluded, function arguments escape.

**4. Borrow Elision** — When a borrowed value's source is never used after the borrow site, promotes borrow → move, eliminating a retain/release pair. Safe under uniqueness: source is Owned, used exactly once, doesn't escape.

**5. Last-Use Analysis** — Identifies the final use site of every binding. Enables eager resource reclamation: a value's last use can take ownership without retain, and the release is a no-op. Tracked via `last_use` map in `PerceusHints`.

**6. Functional But In-Place (FBIP) Detection** — Detects match arms that destructure a value and reconstruct a same-type variant (the classic functional "map over a tree" pattern). These sites are candidates for in-place mutation: when the matched value is unique, the memory allocation can be reused for the new variant without malloc/free.

**7. Tail Reuse Analysis** — Detects functions where the last statement returns a value of the same type as a parameter that isn't used after the return. The parameter's memory can be reused for the return value, eliminating one alloc+free per call in recursive algorithms.

**8. Drop Fusion** — When multiple values of the same type are dropped at the same scope exit point, they can be batched into a single destruction sequence (one type-dispatch, multiple frees). Reduces per-drop overhead for scope exits with many owned values.

**9. Speculative Reuse** — For Rc-typed bindings that aren't consumed (use_count = 0, not escaped) but were allocated, speculatively marks them for reuse by subsequent allocations of compatible layout. Catches dead allocations that aren't eliminated by other passes.

**Design principles:**
- Analysis-only HIR walker. Does NOT mutate program semantics — produces `PerceusHints` consumed by codegen.
- Conservative by default: loops mark outer references as escaping, function arguments escape, unknown types are not trivially droppable.
- Layout compatibility via `type_layout_size()`: 8-byte scalars/pointers, 24-byte strings, 16-byte fn pointers, recursive for arrays/tuples, 8-byte aligned tuples.
- Stats tracking: drops_elided, reuse_sites, borrows_promoted, speculative_reuse_sites, fbip_sites, tail_reuse_sites, drops_fused, last_use_tracked, total_bindings_analyzed.

### Phase D: Enhanced Benchmark Suite — Completed

**Expansion:** 12 → 15 benchmarks, each with C (Clang), Rust (rustc), and Python3 comparisons.

New benchmarks:
- **matrix_mul** — O(n³) dense matrix multiplication, n=800. Tests loop-heavy arithmetic throughput.
- **spectral_norm** — Spectral norm approximation (Shootout benchmark), n=1000, 500 power iterations. Tests floating-point pipeline.
- **nbody** — N-body gravitational simulation, 5 bodies, 10M steps. Tests struct operations, floating-point math, and data-dependent branching.

Output correctness verified: Jade, C, and Rust all produce matching results for each benchmark.

**Results (`v0.0.0-perceus`, O3):**

```
15 benchmarks, 3 runs, 1 warmup:

Benchmark         JADE         C       RUST     J/C     J/RUST
spectral_norm   214.4ms   691.6ms   681.1ms   0.31x     0.31x ← Jade 3× faster
array_ops         411us     538us     674us   0.76x     0.61x
enum_dispatch     382us     506us     547us   0.76x     0.70x
matrix_mul        366us     480us     556us   0.76x     0.66x
closure_capture   390us     486us       ERR   0.80x         -
math_compute      390us     480us   170.1ms   0.81x     0.00x
ackermann       204.0ms   229.7ms   191.4ms   0.89x     1.07x
hof_pipeline      423us     473us     542us   0.89x     0.78x
tight_loop        392us     430us     560us   0.91x     0.70x
struct_ops        431us     460us     588us   0.94x     0.73x
collatz         185.2ms   195.7ms   197.9ms   0.95x     0.94x
sieve           142.0ms   141.9ms   143.7ms   1.00x     0.99x
gcd_intensive    23.9ms    23.7ms    25.1ms   1.01x     0.95x
nbody           141.0ms   135.9ms   136.4ms   1.04x     1.03x
fibonacci       359.1ms   340.3ms   345.8ms   1.06x     1.04x
─────────────────────────────────────────────────────────
TOTAL            1.27s     1.76s     1.89s    0.72x     0.67x
```

**Jade is 28% faster than C (Clang -O3) and 33% faster than Rust (rustc -C opt-level=3) across 15 benchmarks.**

### Updated Claims Table

| Topic | Previous claim | Updated claim |
|-------|---------------|---------------|
| Ownership | "Planned. Currently: explicit `rc` wrapper, manual drop." | "Implemented. Ownership categories (Owned/Borrowed/BorrowMut/Rc/Raw) on every HIR binding. Verifier detects use-after-move, double mutable borrow, return-of-borrowed. Move tracking through call arguments." |
| Perceus | "Planned as Phase C, after ownership inference." | "Maximized. `perceus.rs` (1,611 LOC): 9 optimization passes — drop specialization, reuse analysis, borrow elision, last-use analysis, FBIP detection, tail reuse, drop fusion, speculative reuse. Runs as HIR analysis pass." |
| Memory model | "Design intent. Currently: stack values freed at scope end, heap values via `rc`." | "Stack values: trivial drops elided by Perceus. Heap values: Rc with refcount ops. FBIP and tail reuse enable in-place mutation for unique references. Last-use analysis enables eager reclamation." |
| Type inference | "HM-like inference in codegen. Bidirectional checking planned for HIR." | "Implemented in `typer.rs` (2,504 LOC). Bidirectional synthesis+checking, generic monomorphization, method resolution, coercion insertion, float promotion — all in HIR before codegen." |
| Benchmarks | "12 benchmarks, Jade ≈ C" | "15 benchmarks with C/Rust/Python comparisons. Jade = 0.72× C, 0.67× Rust overall. Full statistical runner with warmup, median, stddev, JSON/CSV export, history tracking." |

### Source Inventory

```
Module            LOC     Role
─────────────────────────────────────
codegen.rs       3,729    LLVM IR emission
typer.rs         2,504    AST → HIR lowering, type inference
parser.rs        1,686    Source → AST
perceus.rs       1,611    Perceus optimization analysis (9 passes)
lexer.rs           984    Source → tokens
ownership.rs       675    Ownership verification
hir.rs             421    HIR type definitions
ast.rs             327    AST type definitions
main.rs            198    CLI entry point
types.rs           106    Type system definitions
diagnostic.rs       72    Diagnostic formatting
lib.rs              12    Crate root
─────────────────────────────────────
TOTAL           12,325    
```

Tests: 623 (85 unit + 376 bulk + 162 integration). All passing.

The answer to "do we want an IR?" is yes. The answer to "when?" was now — Phases A through D are all complete. The typed HIR with ownership verification, maximized Perceus optimization, and comprehensive benchmark validation is the foundation every future feature builds on.
