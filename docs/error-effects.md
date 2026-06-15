# Checked error-effect system

**Status: fully implemented and conformance-tested (2026-06).**
All subsystems are live end-to-end:
- §2 canonical `Option`/`Result` prelude with combinator surfaces
- §3 `err` declaration + raise
- §4 quaternary `e ? ok ! nothing !! err` with `$` / `err` bindings
- §5 implicit propagation, inferred fallibility (SCC least-fixpoint), checked R1-R6
- §6 `From` conversion graph (C1-C4)
- Codegen: quaternary lowering, err-raise early-return, auto-wrap/unwrap, defer on error exits

The deprecated prefix-`!` raise and `?>` operator are removed; the full corpus
is migrated to `err <Variant>` + implicit propagation / quaternary.
27/27 `tests/error_effects.rs` conformance tests pass; full cargo test suite green.

Actor-supervision integration (§8, error propagation to supervision boundary)
depends on structured concurrency — now designed in
[docs/structured-concurrency.md](structured-concurrency.md) (§5 E5) — and is
not yet implemented.

*(Tracked: P1 item in JINN_LANGUAGE_REVIEW_2026_06 §4.5, resolved 2026-06)*

This document specifies Jinn's error model: a canonical `Option`/`Result`
prelude, error declarations and raising with `err`, a *quaternary* expression
that extends the ternary with success/empty/error arms, implicit error
propagation with inferred fallibility, and the inference and conversion rules
that make values-as-errors ergonomic without exceptions.

The design builds on the trait system (task 1-4): error-type conversion is
expressed as a trait (`From`), so propagation across layers is type-directed.

---

## 1. Principles

1. **Errors are values.** No exceptions, no stack unwinding for control flow.
   The error set a function may produce is part of its type.
2. **Inferred by default, checked when annotated.** A function's fallibility
   and its error union are *inferred* from its body. Annotating `! E` narrows
   and documents; it is then checked — the body may not produce an error
   outside `E` (after `From` conversion).
3. **Implicit propagation.** Inside a fallible-capable function, an unhandled
   fallible expression propagates its error outward automatically. The common
   case — write the happy path, let errors flow — costs zero syntax.
4. **One recovery construct.** Handling is the *quaternary*: an extension of the
   ternary `cond ? a ! b` with an error arm `!! c`. Success / empty / error are
   one uniform shape.
5. **Conversion is a trait.** Crossing a layer boundary (`FileError` ->
   `AppError`) uses the `From` trait, applied automatically on propagation.
6. **One canonical `Option`/`Result`.** Shipped in the prelude with combinators.

---

## 2. Prelude types

```jinn
enum Option of T
    Some(T)
    Nothing

enum Result of T, E
    Ok(T)
    Err(E)
```

`Option`/`Result` are ordinary by-value enums: `match`, destructuring, and value
semantics work unchanged. The prelude adds method surfaces (single source of
truth shared by typer + codegen):

Option of T: `is_some()`, `is_none()`, `unwrap()`, `unwrap_or(d)`, `map(*f)`,
`and_then(*f)`, `ok_or(e)`.

Result of T, E: `is_ok()`, `is_err()`, `unwrap()`, `unwrap_or(d)`, `map(*f)`,
`map_err(*f)`, `and_then(*f)`, `ok()`, `err()`.

Combinators are total: none can introduce an undeclared error.

---

## 3. Error declarations and raising: `err`

`err` marks an enum as an error type — a normal enum plus the marker that lets
it appear in an error union and participate in conversion:

```jinn
err FileError
    NotFound
    Denied
```

To **raise** an error, name one of its variants after `err`:

```jinn
*read(p as String)
    if missing(p)
        err NotFound        # raise; this function is now fallible
    bytes(p)
```

`err <Variant>` is a statement that returns early with that error value,
converting it (via `From`, §6) into the enclosing function's error type. The
two roles of `err` are grammatically distinct: `err <Enum>` followed by an
indented variant block *declares*; `err <Variant>` (a known error variant, no
block) *raises*.

A function declares its error union explicitly with a trailing `! E` on the
signature (optional; otherwise inferred):

```jinn
*read(p as String) returns Bytes ! FileError
*sync() ! FileError
```

Semantics of `! E`:
- The function's *result type* is sugar for `Result of R, E`, where `R` is the
  declared `returns` type (or `Unit`). A bare `! E` is `Result of Unit, E`.
- A function with no raised/propagated error has error set `{}` (its result
  type is plain `R`, not a `Result`).
- `! E` is checked (§5): the inferred error set must convert into `E`.

`! E` (signature annotation) and `Result of R, E` (written by hand) are
interchangeable spellings of the same type.

---

## 4. The quaternary: `? ok ! nothing !! err`

The ternary is `cond ? a ! b` (if `cond` then `a` else `b`). The **quaternary**
extends it with an error arm for a fallible subject expression `e`:

```jinn
e ? ok_arm ! nothing_arm !! err_arm
```

- `?`  **success arm** — runs when `e` is `Ok(v)` / `Some(v)`; the unwrapped
  value is available as `$`.
- `!`  **empty arm** — runs when `e` is `Nothing` (Option only; mirrors the
  ternary's else).
- `!!` **error arm** — runs when `e` is `Err(err)`. The error value is available
  as the keyword `err`.

Arms are optional; defaults fill in:

```jinn
x is foo()              # == foo() ? $ !! err   (success => value; error => propagate)
x is foo() !! 0         # == foo() ? $ !! 0     (success => value; error => 0)
foo() ? use($) !! log(err)
find() ? use($) ! none()        # Option: success / empty
```

Multiline arm form (same semantics):

```jinn
foo()
    ? use($)
    !! log(err)
```

`$` is the unwrapped success value, bound only inside a `?` arm (innermost when
nested). `err` is the current error value, usable in a `!!` arm; `!! err`
yields the error itself, which a fallible enclosing function returns — i.e.
explicit propagation. Using `$` outside a `?` arm or `err` outside a `!!` arm is
a compile error.

---

## 5. Propagation and checking

**Implicit propagation.** A fallible expression with no handler propagates: on
success it unwraps to the value; on error the enclosing function returns the
error (after `From` conversion). `foo()` is exactly `foo() ? $ !! err`.
Propagation makes the enclosing function fallible; its inferred error union
grows to include the propagated error (after conversion). Fallibility is thus
inferred, never declared involuntarily.

This holds uniformly across positions: a bound `v is foo()`, a bare
statement `foo()`, and a fallible store mutation `insert s a, b` all propagate
the same way inside a fallible function. A bare `insert s a, b` therefore needs
no `? $ !! err` ceremony — it propagates a store error (converted via `From`)
and yields the insert's result, and as a tail expression auto-wraps to `Ok`.
Inside a non-fallible function (including `main`), a store mutation with no
handler is fire-and-forget: its error is discarded, per R4.

Let `decl(f)` be the declared error union (from `! E`, if present) and `inf(f)`
the inferred union (from the body). The typer computes `inf(f)` and checks:

R1. **Soundness of declaration.** Every error enum in `inf(f)` must be
    convertible (§6) into some enum in `decl(f)`. Otherwise: "function declares
    `! E` but may produce `X`, and no conversion `X -> E` exists".

R2. **No silent widening.** `! E` never adds errors the body cannot produce;
    it is an upper bound. Inferred-only functions get `decl = inf`.

R3. **Propagation well-formedness.** A propagated `e` must be `Result`/`Option`,
    and a conversion `X -> E` into the enclosing error type must exist (else
    R1-style error at the propagation site).

R4. **`main`.** `main` is not fallible by default; an error propagated out of
    `main` causes a nonzero process exit and the runtime prints the error. No
    ceremony is required for scripts.

R5. **Exhaustive handling.** `match` on a `Result`/`Option` must cover all
    variants (existing exhaustiveness checker applies).

R6. **`err`-raise consistency.** `err X` inside `f` requires `X` convertible
    into `decl(f)` via the conversion graph (§6).

---

## 6. Conversion: the `From` trait

```jinn
trait From of S
    *from(s as S) returns Self
```

Propagation and `err`-raise insert `E.from(x)` when the produced error type `X`
differs from the target `E` and an `impl From of X for E` exists.

C1. **Reflexivity.** `X -> X` is always available (identity); no impl needed.
C2. **One step.** Conversion is a single `From` application, not transitive.
C3. **Ambiguity.** If multiple target enums in a union can receive `X`,
    disambiguation is required. Single-target unions are unambiguous.
C4. **Coherence.** `impl From of X for E` is allowed only in the module defining
    `X` or the module defining `E` (orphan rule).

---

## 7. Inference algorithm

During HIR lowering of a function body (extends `current_fn_error_types`):

1. Seed `inf = {}`.
2. For each `err X` raise: add `X`'s enum to `inf`.
3. For each propagated fallible call `g(..)` of result `Result of _, Eg`: add
   each member enum of `Eg` to `inf`.
4. Handled (matched / quaternary-handled with a non-propagating `!!` arm)
   results do *not* contribute to `inf`.
5. Recursion / SCCs: error sets are computed to a least fixpoint over the call
   graph SCC (reuse `src/typer/scc.rs`); terminates because the enum universe
   is finite.
6. If `decl` present: check R1; the externally visible error type is `decl`.
   Else it is `inf`.

---

## 8. Interaction with existing features

- **`defer`** runs on both success and propagated-error exits. Perceus drops
  are emitted on the propagation early-exit path.
- **Generators** may be fallible; `err`-raise / propagation returns from the
  generator.
- **Actors/channels**: a fallible handler propagates to the supervision
  boundary (designed: [docs/structured-concurrency.md](structured-concurrency.md) §5 E5).
- **Value semantics:** `Option`/`Result` are by-value enums; no GC implications.

---

## 9. Diagnostics

- Undeclared produced error (R1): point at the `err X` / propagation site and
  the signature; fix-it: add `| X` to the union or `impl From of X`.
- Missing conversion (C-rules): "no conversion `X -> E`"; fix-it: scaffold
  `impl From of X for E`.
- `$` outside a `?` arm / `err` outside a `!!` arm: point at the use.

---

## 10. Surface syntax summary

```jinn
err NetError
    Timeout
    Refused

err FileError
    NotFound
    Denied

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

*backup(url as String, path as String) returns Bytes ! AppError
    data is fetch(url)              # NetError -> AppError via From, propagated
    save(path, data)               # FileError -> AppError via From, propagated
    data                           # bare value auto-wraps to Ok

*main()
    cfg is load("a.cfg") !! default_config()     # error => fallback value
    load("a.cfg") ? use($) !! log(err)           # success / error handlers
```
