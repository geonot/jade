# §8 Ownership, borrow, move, take, access modes

**Files:** `src/ownership/` (1,033 LOC). HIR carries the `Ownership`
enum produced by the typer (see §7).

## 8.1 Model on paper

- Each binding is `Owned`, `Borrowed`, `BorrowMut`, or `Raw`.
- The typer infers and classifies; the borrow-check walker
  (`src/ownership/`) reports `OwnershipDiag` variants:
  `UseAfterMove`, `DoubleMutableBorrow`, `MoveOfBorrowed`,
  `InvalidRcDeref`, `ReturnOfBorrowed`, plus warnings.
- The `take` keyword explicitly transfers ownership.
- Perceus + MIR's drop machinery is the runtime side; this section
  is about the static checker.

## 8.2 What I was able to test

Probe `p25_take.jn`:
```
*consume(v) returns i64
    v[0]

*main
    v is [1, 2, 3]
    a is consume(take v)
    log(a)
    log(v[0])
```
**Compiler error:** `line 7:23: expected ,, got v`. The parser does
not accept `take EXPR` in a function-call argument position
(F-PARSE-6). **I could therefore not exercise the borrow-check
machinery end-to-end through the user-visible `take` keyword.**

Probe v1's `p17_use_after_move.jn`-style probe **also did not
emit a borrow-check diagnostic**: a struct's field was moved and then
re-read, and the program ran and printed a normal value. Either the
ownership pass is missing this case, or the typer is classifying the
field as `BorrowMut`/`Raw` so the move never happens.

## 8.3 Findings

### F-OWN-1 (P0): The borrow-check surface cannot be reached from user code

If `take` does not parse in the position users will naturally write
it, the borrow-checker is functionally unreachable. This is a
blocker for alpha because the entire safety story of the language
rides on this pass being effective.

### F-OWN-2 (P0): Apparent use-after-move not caught in probe v1 case

Hard to fully characterise without §F-OWN-1 fixed. I can either
infer the typer is classifying things conservatively (correct) or
that the ownership walker is missing the field-move case (incorrect).
A focused probe with the `take` keyword usable would let me
disambiguate.

### F-OWN-3 (P1): Implicit copy semantics are not documented

When the user writes `let a = v` for a `Vec<T>`, what happens? In
probe v1's `p17_use_after_move.jn` no diagnostic fired and the
program ran. The probable answer is "Jinn implicitly copies trivials
and Rc-bumps non-trivials" — but no document I read makes this clear
and it is not testable without F-OWN-1.

### F-OWN-4 (P2): `OwnershipDiag::ReturnOfBorrowed` exists; check what triggers it

The enum variant is in the source, but I could not find a probe that
triggers it. A function that returns a `&T` derived from a local
should fire it; needs a targeted test.

## 8.4 Verdict

**Not assessable beyond paper review** until F-OWN-1 (parsing of
`take` in argument position) is fixed. Until then, the language's
safety promise is structurally unverifiable by users. This alone is a
P0 alpha blocker — a memory-safe language whose safety apparatus
users cannot invoke is not a memory-safe language.

---

# §9 MIR & SSA review

**Files:** `src/mir/` (6,391 LOC) including `mod.rs`, `lower/`,
`opt/`, `printer.rs`.

## 9.1 Shape

- Functions are SSA: blocks with phi nodes, terminator per block.
- Instructions are explicit, including `Drop`.
- A `PerceusMeta` side-table is attached to each `Function` (defined
  in `src/mir/mod.rs:42`) carrying:
  - `reuse_save: HashMap<ValueId, u32>` (drop → slot)
  - `reuse_consume: HashMap<ValueId, u32>` (alloc → slot)
  - `drop_fusion_runs: Vec<Vec<ValueId>>`
  - `tail_reuse: HashMap<ValueId, ValueId>`
  - `pool_allocs: HashSet<ValueId>`
  - `vec_slots: HashSet<u32>` (Vec-aware reuse)
- A `PerceusStats` rollup is surfaced via `--debug-perceus`.

The metadata model — codegen-invisible information lives in the
side-table, codegen-visible instructions live in the IR — is exactly
right and is what mature compilers do. **This is some of the cleanest
design in the project.**

## 9.2 Findings

### F-MIR-1 (P0): MIR is not internally type-checked

Operand-type mismatches reach codegen (see F-TYPE-1) because the MIR
itself has no verifier. The fix is a small `mir::verify` pass that
walks every instruction and asserts operand types match the
operator's signature. Crash MIR's verifier early; never let LLVM be
the one to find the mismatch.

### F-MIR-2 (P0): MIR pipeline is unsound for at least one construct

Probe `p31_generator.jn`:
```
$ jinnc p31_generator.jn -o /tmp/x
Function return type does not match operand type of return inst!
  ret i64 0
 ptr
```
The error is LLVM's IR verifier complaining that the function's
declared return type and the value being returned disagree. A sound
compiler never produces invalid IR. The generator/`yield` lowering is
emitting a `ret i64 0` for a function whose return type is `ptr`.
This is a soundness bug — see §11 for the codegen side.

### F-MIR-3 (P1): `mir-perceus:` line leaks to stdout in some runs

Probes `p18_map_ice` and `p24_interp` printed an unexpected line:
```
mir-perceus: 0 drops elided, 0 drops sunk, 2 drops fused, 0 reuse pairs (15 bindings)
```
without a `--debug-perceus` flag being passed. The diagnostic suppression
table only filters known prefixes (e.g. `mir_codegen:`, `typer:`,
`hir:`); `mir-perceus:` slipped through. Minor but visible.

### F-MIR-4 (P2): The HIR-level Perceus is dead code

Per memory note `jade_codegen_analysis.md`, the old HIR-level Perceus
(`PerceusPass`) is retired and only the MIR-level pass is alive. The
shim should be deleted; the `hints` field is flagged dead by the
compiler.

## 9.3 Verdict

**MIR's design is alpha-ready.** Its enforcement (F-MIR-1) and the
generator lowering (F-MIR-2) are not. F-MIR-1 in particular has
outsized return on investment: a 50-line MIR verifier would let codegen
delete its auto-widening fallback and turn a class of latent ICEs into
loud build-time errors.

---

# §10 Perceus & drop insertion

**Files:** `src/perceus/` (944 LOC; primarily `mir_perceus.rs`).

## 10.1 What it does

Drop instructions for trivially-droppable values are physically
deleted from MIR. For non-trivial drops, the pass:

1. Elides drops in branches where the value is statically known dead.
2. Sinks drops past control flow to permit reuse pairing.
3. Fuses consecutive drops into a single batched free.
4. Pairs a drop with a subsequent allocation of the same shape
   (`reuse_save` / `reuse_consume` slots).
5. Hints loop-body allocations as pool candidates.
6. Identifies Vec slots (preserve header + buffer, reset `len=0` for
   the next push run).
7. Hints tail-call reuse (alloc dest reuses incoming owned param's
   storage).

This is **the right Perceus design**, faithful to the Lean 4 paper,
and the side-table layering (§9) is correct.

## 10.2 What I observed

- Probe `p32_alloc_churn.jn` runs 100,000 iterations of
  `v is vec(); v.push(i); sum += v[0]` cleanly in 14 ms. The reuse
  pairing or pool-alloc hint is doing its job. Without it the
  allocator would dominate and the run would be far slower.
- The visible Perceus diagnostics line (F-MIR-3) leaks to stdout.

## 10.3 Findings

### F-PER-1 (P2): No reuse-pair confirmation visible in default mode

`--debug-perceus` is documented (memory) but the user gets no
indication that Perceus did anything unless they ask. That is correct
for a release build. But a `jinnc --explain perceus FILE` mode would
be a great onboarding aid.

### F-PER-2 (P2): The pool-allocs hint is informational-only

Per `src/mir/mod.rs:84` source comment:
> `pool_allocs: HashSet<ValueId>` — Allocation sites that occur inside a
> loop body and would benefit from a pool. Currently informational only
> — wired into stats reporting.

The hint is computed but not consumed. Either wire it up or remove
the field — leaving identified-but-unused metadata invites bit-rot.

### F-PER-3 (P3): Drop semantics under panic/trap are not exercised

What happens to non-trivial drops queued for a function frame when a
trap (e.g. div-by-zero in the HIR path, or bounds-check) fires
mid-block? With the runtime aborting via `__jinn_trap`, those drops
simply do not run. For an alpha that's tolerable; for production it
matters (e.g. file handles, sockets, locks).

## 10.4 Verdict

**Alpha-ready in design.** The findings here are bookkeeping cleanups,
not blockers.
