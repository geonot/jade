# Stability & Compatibility Contract

This document defines the stability tiers Jinn uses and the guarantees each
tier carries. It applies to the **language**, the **compiler/runtime
interfaces**, and the **standard library** (`std/`). The per-module stdlib
catalogue lives in [`docs/std.md`](std.md).

## Pre-1.0 status

Jinn is pre-1.0 (current crate version `0.0.0`). During the alpha series the
language and standard library are still moving. The tiers below describe the
*intent* and the *enforced* guarantees today; full semantic-versioning
guarantees begin at the first stable (`1.0`) release.

## Tiers

### Stable

A **stable** surface is expected to keep compiling and to keep its observable
behaviour across patch releases within the same minor series.

- **Language:** the core syntax and semantics exercised by the test suite
  (`tests/`) and the example programs (`apps/`, `benchmarks/`) — functions
  (`*name`), `type`/`enum` declarations, pattern `match`, the expression and
  statement grammar, actors/channels, and the `store` surface.
- **Standard library:** the **alpha-stable subset** enumerated in
  [`docs/std.md`](std.md). Membership is *machine-checked*: every module in the
  subset must pass the compiler frontend
  (`jinnc std/<module>.jn --lib --emit-hir`) via the
  `std_stable_subset_frontend_checks` test in
  [`tests/std_stable_subset.rs`](../tests/std_stable_subset.rs). A regression in
  any stable module fails CI.

Stability at the alpha tier guarantees the surface **type-checks against the
current language**. API stability across *minor* versions is a stronger bar
that is raised at `1.0`.

### Experimental

An **experimental** surface may change or be removed without notice. It is not
covered by the compatibility guarantees above.

- **Language:** features that are partially implemented or behind active design
  (for example, `try`/`rescue` exception handling, which currently has no
  lexer/parser/HIR support).
- **Standard library:** any module **not** in the alpha-stable subset. These are
  tracked in the `EXPERIMENTAL` list in
  [`tests/std_stable_subset.rs`](../tests/std_stable_subset.rs) with a written
  justification, and the gate is **self-policing**: the
  `std_experimental_list_is_accurate` test asserts each excluded module *still
  fails* the frontend check, so a module cannot silently remain experimental
  once the blocking feature lands.

### Deprecated

A **deprecated** surface still works but is scheduled for removal. Deprecations
are announced in the changelog for the release that introduces them and are
removed no earlier than the next minor release. Where practical the compiler
emits a deprecation diagnostic pointing at the replacement.

## How stability is enforced

| Surface | Mechanism |
| --- | --- |
| Stable stdlib subset | `tests/std_stable_subset.rs` (`--lib --emit-hir` gate, run in `cargo test`) |
| Experimental accuracy | `std_experimental_list_is_accurate` (excluded modules must still fail) |
| Language/runtime behaviour | `tests/` integration + audit suites (`tests/alpha_release_audit.rs`) |
| Formatting | `cargo fmt --check` |

## Changing a tier

1. Promoting a module to stable: it must pass the frontend gate; remove its
   `EXPERIMENTAL` entry in the same change that makes it compile.
2. Demoting or removing a stable surface before `1.0`: allowed during alpha, but
   must be recorded in the changelog.
3. Adding a new experimental surface: permitted, but it must be documented and —
   for stdlib — paired with a tracked language-feature gap.
