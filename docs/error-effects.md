# Checked error-effect system

Status: design (P1, JINN_LANGUAGE_REVIEW_2026_06 §4.5)

This document specifies Jinn's error model: a canonical `Option`/`Result`
prelude, a checked error-effect declaration (`! E`), an error-propagation
operator, and the inference and conversion rules that make values-as-errors
ergonomic without exceptions.

The design builds on the now-landed trait system (bounded polymorphism, task
1-4): error-type conversion is expressed as a trait, so propagation across
layers is type-directed rather than hand-rolled.

---

## 1. Principles

1. **Errors are values.** No exceptions, no stack unwinding for control flow.
   The error set a function may produce is part of its type.
2. **Checked, not advisory.** A declared error union `! E` is enforced: the
   compiler proves the function produces no error outside `E`, and that every
   produced error is either propagated to a declarer or handled.
3. **Inference over ceremony.** A function that does not annotate `! E` has its
   error union *inferred* from its body. Annotation narrows/documents; it never
   adds errors that are not produced.
4. **Conversion is a trait.** Crossing a layer boundary (`FileError` ->
   `AppError`) uses the `From` trait, applied automatically by the propagation
   operator. No bespoke per-boundary match cascades.
5. **One canonical `Option`/`Result`.** Shipped in the prelude with combinators,
   so the ecosystem does not fragment into N incompatible result types.

---

## 2. Prelude types

Defined in the prelude (`std`/auto-imported), monomorphized like any generic
enum.

```jinn
enum Option of T
    Some(T)
    Nothing

enum Result of T, E
    Ok(T)
    Err(E)
```

The empty Option variant is spelled `Nothing` (it is also what the iterator
protocol yields). `is_none()` is provided as a synonym for `is_nothing()` on the
combinator surface.

`Option` and `Result` are ordinary enums: `match`, destructuring, and value
semantics all work unchanged. The prelude adds method surfaces (single source of
truth shared by typer + codegen, per project convention):

Option of T:
- `is_some() returns bool`, `is_none() returns bool`
- `unwrap() returns T`            (traps on `None`)
- `unwrap_or(d as T) returns T`
- `map of U(*f) returns Option of U`
- `and_then of U(*f) returns Option of U`
- `ok_or of E(e as E) returns Result of T, E`

Result of T, E:
- `is_ok() returns bool`, `is_err() returns bool`
- `unwrap() returns T`            (traps on `Err`)
- `unwrap_or(d as T) returns T`
- `map of U(*f) returns Result of U, E`
- `map_err of F(*f) returns Result of T, F`
- `and_then of U(*f) returns Result of U, E`
- `ok() returns Option of T`, `err() returns Option of E`

Combinators take `*f` (function values) per Jinn's `*name = fn` sigil. They are
total: no combinator can introduce an undeclared error.

---

## 3. Error declarations: `err` and `! E`

`err` marks an enum as an error type. It is a normal enum plus the marker that
lets it appear in an error union and participate in conversion:

```jinn
err FileError
    NotFound
    Denied
```

A function declares the errors it may produce with a trailing `! E`. The union
may list several error enums:

```jinn
*read(path as String) returns i64 ! FileError
*sync() ! FileError | NetError
```

Semantics of `! E1 | E2 | ...`:
- The function's *result type* is sugar for `Result of R, E` where `R` is the
  declared `returns` type (or `Unit`) and `E` is the union of the listed error
  enums. A bare `! E` with no `returns` is `Result of Unit, E`.
- A function with no `!` and no error-producing operations has error set `{}`
  (its result type is plain `R`, not a `Result`).
- The error set is part of the function's type and is checked (§5).

This unifies the two surfaces the language already exposes (`Result of T, E`
written by hand, and `returns R ! E`): `! E` *is* `Result`, surfaced with less
ceremony. Code may use either spelling interchangeably.

---

## 4. Propagation operator `?>`

The review (§4) warns that `?` is already overloaded (ternary-else, match arm).
To avoid a third meaning, propagation uses a distinct two-character operator
`?>` ("propagate"), a postfix operator on an expression of `Result`/`Option`
type:

```jinn
*load(path as String) returns Config ! ConfigError
    raw is read(path)?>          # read returns i64 ! FileError
    parse(raw)?>                 # parse returns Config ! ParseError
```

Reading `e?>`:
- If `e` is `Ok(v)` / `Some(v)`, the whole expression evaluates to `v`.
- If `e` is `Err(err)` / `None`, the enclosing function returns early with the
  error, after converting `err` into the enclosing function's error type via
  `From` (§6).

`?>` is only valid inside a function whose result type is a `Result`/`Option`
(equivalently, that declares `! E`). Using it elsewhere is a compile error with
the fix-it "declare the enclosing function's error union with `! E`".

`!` (early-return) is retained and unchanged: `! NotFound` is the explicit
"return this error now" form. `?>` is the implicit "propagate the error in this
value" form. The two compose: `?>` desugars to a `match` that uses `!` on the
error arm.

Desugaring of `read(path)?>` in a function declaring `! ConfigError`:

```jinn
match read(path)
    Ok(v) ? v
    Err(e) ? ! ConfigError.from(e)
```

For `Option` in a function declaring `! E` with `E: FromNone` (or returning
`Option`), `None` propagates analogously; in a `Result` context `None` requires
an `ok_or` first (the compiler suggests it).

---

## 5. Checking rules

Let `decl(f)` be the declared error union of `f` (from `! E`), and `inf(f)` the
inferred union (from the body). The typer computes `inf(f)` and checks:

R1. **Soundness of declaration.** Every error enum in `inf(f)` must be
    convertible (§6) into some enum in `decl(f)`. If `f` declares `! E` and the
    body can produce `X` with no `From` path `X -> E`, error:
    "function declares `! E` but may produce `X`, and no conversion `X -> E`
    exists".

R2. **No silent widening.** A declared `! E` never *adds* errors the body cannot
    produce; declaration is an upper bound used for documentation and for the
    target type of conversion. Inferred-only functions get `decl = inf`.

R3. **Propagation well-formedness.** `e?>` requires:
    - `e : Result of _, X` or `Option of _`,
    - the enclosing function has a result error type `E`,
    - a conversion `X -> E` exists (`From`, §6), else error R1-style at the
      `?>` site.

R4. **Caller obligation.** A `Result`-returning call whose value is neither
    propagated (`?>`), matched, bound and later handled, nor explicitly
    discarded (`_ is call()`), is a warning "unhandled error result"
    (escalatable to error under `--strict-errors`). This is the teeth the
    review asked for: declared errors must be handled.

R5. **Exhaustive handling.** `match` on a `Result`/`Option` must cover all
    variants (existing exhaustiveness checker applies unchanged).

R6. **`!` consistency.** `! X` inside `f` requires `X` convertible into
    `decl(f)` (already partially enforced; extended to use the conversion graph
    rather than exact-membership).

---

## 6. Conversion: the `From` trait

Error conversion is type-directed, via a prelude trait:

```jinn
trait From of S
    *from(s as S) returns Self
```

`?>` and `!` insert `E.from(x)` when the produced error type `X` differs from
the target `E` and an `impl From of X for E` exists. Rules:

C1. **Reflexivity.** `X -> X` is always available (identity); no impl needed.
C2. **One step.** Conversion is a single `From` application, not a transitive
    search, to keep inference decidable and errors local (matches Rust's `?`).
C3. **Ambiguity.** If multiple distinct target enums in a union `E1 | E2` can
    receive `X`, the compiler requires disambiguation (annotate or convert
    explicitly). Single-target unions are unambiguous.
C4. **Coherence.** `impl From of X for E` is allowed only in the module
    defining `X` or the module defining `E` (orphan rule), preventing
    conflicting conversions.

A union target `! E1 | E2` accepts `X` if `From of X` is implemented for exactly
one `Ei`.

---

## 7. Inference algorithm

During HIR lowering of a function body (extends current
`current_fn_error_types`):

1. Seed `inf = {}`.
2. For each `! X` statement: add `X`'s enum to `inf`.
3. For each call `g(..)` whose result is a `Result of _, Eg` and is propagated
   with `?>`: add `Eg` to `inf` (each member enum of `Eg`).
4. For each `Result`-typed expression matched/handled locally: does *not*
   contribute to `inf` (handled, not propagated).
5. Recursion / SCCs: error sets are computed to a fixed point over the call
   graph SCC (reuse `src/typer/scc.rs`), monotone least fixpoint, terminates
   because the enum universe is finite.
6. If `decl` present: check R1 (every member of `inf` converts into `decl`).
   Then the function's externally visible error type is `decl`. Else it is
   `inf` (and is reported in diagnostics / hover).

This makes `! E` a real, checked effect: declared error sets are upper bounds
verified against an inferred lower bound, with conversion mediating the gap.

---

## 8. Interaction with existing features

- **`defer`** runs on both `Ok` and propagated-`Err` exits (it already runs on
  any function exit). Perceus refcount drops are emitted on the `?>` early-exit
  path like any other early return.
- **Generators** (`is_generator`) may declare `! E`; a yielded sequence that can
  fail uses `Result` element types; `?>` inside a generator returns from the
  generator.
- **Actors/channels** (concurrency tasks 2-5..2-9): a fallible actor message
  handler declares `! E`; `?>` propagates to the actor's supervision boundary
  (specified in the structured-concurrency design, task 2-5).
- **Value semantics:** `Option`/`Result` are by-value enums; no aliasing or GC
  implications beyond ordinary enum payload moves.

---

## 9. Diagnostics (must-have quality)

- Unhandled error result (R4): point at the call, list the unhandled error
  enum, fix-its: add `?>`, `match`, or `_ is`.
- Undeclared produced error (R1): point at the `! X` / `?>` site and the
  function signature; fix-it: add `| X` to the union or add `impl From of X`.
- `?>` outside fallible fn (R3): point at the `?>`; fix-it: add `! E` to the
  enclosing signature.
- Missing conversion (C-rules): "no conversion `X -> E`"; fix-it: scaffold
  `impl From of X for E`.

---

## 10. Implementation phasing (subtasks)

- 2-2 prelude `Option`/`Result` + combinator method surfaces.
- 2-3 `?>` parse + desugar to `match`/`!`, using `From`.
- 2-4 checking rules R1-R6, SCC error-set fixpoint, `From` conversion graph,
  caller-obligation warning, `--strict-errors`.

Conformance suite (`tests/error_effects.rs`) pins: prelude combinators;
`?>` happy/early-exit; declared-vs-inferred soundness error; missing-conversion
error; unhandled-result warning; `From`-mediated cross-layer propagation;
ambiguous-union diagnostic; `defer` + `?>` ordering.

---

## 11. Surface syntax summary

```jinn
err NetError
    Timeout
    Refused

err AppError
    Io(FileError)
    Net(NetError)

impl From of FileError for AppError
    *from(e as FileError) returns AppError
        AppError.Io(e)

impl From of NetError for AppError
    *from(e as NetError) returns AppError
        AppError.Net(e)

*fetch(url as String) returns Bytes ! NetError
*save(path as String, b as Bytes) ! FileError

*backup(url as String, path as String) ! AppError
    data is fetch(url)?>        # NetError -> AppError via From
    save(path, data)?>          # FileError -> AppError via From
    Ok(Unit)
```
