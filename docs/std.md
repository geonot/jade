# The Jinn Standard Library

The standard library lives in `std/` and is consumed with `use`:

```jinn
use math
use strings

*main
    r is math.sqrt(2.0)        # cross-module calls are QUALIFIED
    s is strings.to_upper('hi')
    log(r)
    log(s)
    0
```

Conventions:

- **Functions are called qualified** by module name: `convert.parse_int(...)`.
- **Types are global / unqualified**: a `TcpStream` from `net` is written
  `TcpStream`, not `net.TcpStream`.

For the language-level stability contract (tiers and what each guarantees) see
[`docs/stability.md`](stability.md). This document defines the **alpha-stable**
subset of `std/` and the policy that governs it.

## What "alpha-stable" means

A module is **alpha-stable** when it passes the compiler frontend
(lex → parse → type-check → HIR lowering) in library mode:

```sh
jinnc std/<module>.jn --lib --emit-hir
```

This contract is enforced automatically by the
`std_stable_subset_frontend_checks` test in
[`tests/std_stable_subset.rs`](../tests/std_stable_subset.rs), which runs as
part of `cargo test` in CI. A regression in any stable module fails the build.

Alpha-stable guarantees the module **type-checks against the current language**.
It is not yet a guarantee of API stability across versions — that bar is raised
post-alpha.

## Alpha-stable modules (50)

| Domain | Modules |
| --- | --- |
| Core / language | `convert`, `fmt`, `bytes`, `bit`, `binary`, `volatile`, `collections`, `sort` |
| Numerics | `math`, `complex`, `rational`, `decimal`, `bigint`, `stats`, `fft`, `random` |
| Text | `strings`, `regex`, `glob`, `codec`, `hex`, `uuid` |
| Data formats | `json`, `csv`, `toml`, `dataframe` |
| Crypto | `crypto`, `aes`, `argon`, `blake`, `sha`, `tls` |
| I/O & OS | `io`, `fs`, `path`, `os`, `args`, `process`, `signal`, `terminal`, `logging` |
| Networking | `net`, `http`, `url`, `bangle` |
| Time | `time`, `date` |
| Storage | `sqlite` |
| Concurrency / systems | `event`, `raft` |

## Experimental modules (excluded)

These modules depend on language features that are not yet implemented and are
**not** part of the alpha-stable subset. They are tracked in the `EXPERIMENTAL`
list in [`tests/std_stable_subset.rs`](../tests/std_stable_subset.rs).

| Module | Reason |
| --- | --- |
| `test` | Uses `try`/`rescue` exception handling, which has no lexer/parser/HIR support yet. |

The gate is self-policing: the `std_experimental_list_is_accurate` test asserts
that every excluded module *still fails* the frontend check. When the blocking
feature lands and the module compiles, that test fails until the entry is
removed from `EXPERIMENTAL` — at which point the module joins the stable subset
and is gated like the rest.

## Policy

1. New modules added to `std/` are expected to pass the frontend gate.
2. A module may only be added to `EXPERIMENTAL` with a written justification and
   a tracked language-feature gap.
3. When a language feature lands, the corresponding experimental entry must be
   removed in the same change that makes the module compile.
