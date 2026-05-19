# §6 AST & HIR review

**Files:** `src/ast/`, `src/hir/`, `src/hir_validate.rs` (≈ 1.4 k LOC for HIR).

## 6.1 Shape

- AST has full source span on every node (`Span { file: Option<Symbol>,
  line: u32, col: u32 }`). Spans propagate through HIR and into
  diagnostics.
- HIR introduces `DefId(u32)` — a flat numeric identity for any
  top-level definition (function, type, actor, store, etc.). This is
  the right model.
- HIR carries an `Ownership` enum (`Owned | Borrowed | BorrowMut | Raw`)
  on bindings; the typer is responsible for setting it. This is the
  primary surface the borrow-check pass (`src/ownership/`) consumes.

## 6.2 Findings

### F-HIR-1 (P1): `hir_validate` is the runtime-of-last-resort type check

As called out in §5 (F-PARSE-7) and §7 — the typer accepts programs
that have type-coherence violations and HIR-validate catches them.
The validator does its job, but its existence is a signal that the
typer is leaky. Once §7's typer remediation is done, `hir_validate`
should reduce to a small invariant checker, not a working type
checker.

### F-HIR-2 (P2): Ownership enum is shallow

`Ownership` is a four-state flag on a binding. For aggregate types
this is not enough: a value can have *parts that are owned* and
*parts that are borrowed* (a struct with one owned `Vec<T>` field and
one `&str` field). The typer's `moved_fields` machinery papers over
this but the HIR doesn't represent it directly. For alpha-tier
ownership diagnostics, this is OK; for beta this needs to grow to a
per-projection ownership.

### F-HIR-3 (P3): No `Validated` newtype to track HIR↔Validated state

A common pattern in mature compilers is to express "this HIR has been
validated" at the type level so subsequent passes cannot accidentally
consume unvalidated HIR. Jinn doesn't, which means the discipline is
purely social. Not blocking; nice-to-have.

## 6.3 Verdict

**Alpha-ready as a representation.** The findings here are about the
*producer* (typer) not the *representation* itself.

---

# §7 Type system, inference, generics

**Files:** `src/typer/` (15,879 LOC across resolve, infer, monomorph,
ownership-classify, hir-lower).

This is the largest single area of architectural risk in the project.

## 7.1 Findings

### F-TYPE-1 (P0): The typer accepts type errors that codegen later trips on

The `mir_codegen::helpers::values` auto-widening fallback for
mismatched integer widths bears this comment in source:

> // Auto-widen mismatched integer widths to the wider operand.
> // Required because MIR currently lets `i32 << i64` reach codegen
> // unaltered (e.g. `i * 2` where `i: i32` and `2: i64`); LLVM
> // rejects this with "operands not of the same type".

Translation: **the typer permits operand-type mismatches that LLVM
would reject, and codegen patches them.** This is a textbook
"convenient fix" instead of "right fix" — the right fix is for the
typer to insert explicit `Cast` nodes during inference so that MIR is
well-typed at the operand level. Then `helpers/values.rs:91-118` can
delete the entire auto-widen block.

### F-TYPE-2 (P0): `Type::String` vs `Type::Struct(Symbol("string"), [])` asymmetry

(Per memory note `jade_type_inference.md`.) The string type is
represented two different ways: as a primitive variant `Type::String`,
and as a degenerate struct named `string`. Pattern matching and
equality on `Type` doesn't unify these. The trait probe
`'woof: ' + self.name` works in probe `p16_trait.jn` only because the
two happen to align on the relevant code path; deeper trait probes
would split on this.

**Fix:** Pick one. Either `Type::String` is the canonical form and
the `Struct("string")` shadow goes away, or vice versa. Add a
`Type::canonicalize()` and have it called at every comparison site.

### F-TYPE-3 (P0): Generic inference for closure body `$ * 2` loses element type

Probe `p18_map_ice.jn`:
```
v is [1, 2, 3, 4, 5]
d is map(v, $ * 2)
log(d[0])
```
Probe v1 panicked the compiler at
`src/codegen/mir_codegen/helpers/values.rs:96` with `PointerValue
where IntValue expected`. Probe v2 (same logical program) compiles
but exits with rc=16 (subprocess SIGFPE or aborted multiply).

The typer is inferring the closure's `$` parameter as `Ptr<i64>` (the
slot pointer the codegen feeds in) rather than `i64`. The codegen
then attempts integer multiplication on a pointer value, and either
ICEs (v1) or emits a multiply that LLVM compiles into an `sdiv`-like
poison that the verifier passes but the runtime rejects.

**This is the most damaging bug in the project**: it's the exact
"first-hour" idiom a user will reach for, and it produces either a
compiler crash or a runtime crash with no useful diagnostic.

### F-TYPE-4 (P1): The typer struct has ~50 fields

`src/typer/` is one big mutual collaboration. Adding a feature
requires touching state across resolve, infer, monomorph, escape, and
hir-lower. The risk surface is enormous. This is a classic "god
object" — Linus would split it into three: a `Resolver`, a
`TypeInferrer`, and a `HirBuilder`, each owning a coherent slice of
state and communicating through explicit input/output types.

### F-TYPE-5 (P2): Untyped lambdas work, typed lambdas required in some contexts

Probes `p10_typed_lambda.jn` and `p10b_untyped_lambda.jn` both work
for the trivial cases. But the memory note `jade_type_inference.md`
recorded that some lambda contexts require explicit `|x as i64|` to
type-check, and the inability of `map(v, $ * 2)` to infer element
type (F-TYPE-3) confirms inference is incomplete in HOF contexts.

### F-TYPE-6 (P2): Generic functions monomorphise eagerly

`*idfn(x) is x` and `idfn(42); idfn('hi'); idfn(3.14)` work
(p26_generic). The typer is monomorphising the function for each
call. This is fine for alpha but has known scalability cost (compile
time growth and code bloat). Track as a beta-level concern.

### F-TYPE-7 (P3): Error type unions surface as `! Variant` sugar

The `is x() ! Variant` and `! Variant` standalone statement are
Jinn's answer to Rust's `?` operator. The desugaring is in the
parser (see F-PARSE-10) and the typer infers the union. The model
works but is not documented in the EBNF in a way that distinguishes
the standalone-statement form from the trailing-bang form, which is
why probe v1 confused them.

## 7.2 What's correct

- Generic monomorphisation actually works across primitive types.
- Trait dispatch (probe `p16_trait.jn`) emits the right method.
- Pattern exhaustiveness is checked (verified by probe v1).
- Tail-call optimization is applied (probe `p41_tco.jn` runs a
  10-million-deep tail call to completion).
- Mutual recursion compiles and produces the right answer
  (`p34_mutual.jn`).

## 7.3 Verdict

**Not alpha-ready.** F-TYPE-1, F-TYPE-2 and F-TYPE-3 are blocking.
F-TYPE-4 is the architectural lift that will keep biting until it's
done. Plan in §24.
