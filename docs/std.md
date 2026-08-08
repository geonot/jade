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

A module is **alpha-stable** when it clears two bars. First, it passes the
compiler frontend (lex → parse → type-check → HIR lowering) in library mode,
which covers every function in the module including ones no caller reaches:

```sh
jinnc std/<module>.jn --lib --emit-hir
```

Second, a program that imports it compiles all the way through codegen, links
against the runtime, and runs:

```sh
printf 'use std/<module>\n\n*main\n    log(1)\n    0\n' > importer.jn
jinnc importer.jn -o importer && ./importer
```

Both contracts are enforced automatically, by
`std_stable_subset_frontend_checks` and `std_stable_subset_imports_and_links`
in [`tests/std_stable_subset.rs`](../tests/std_stable_subset.rs), which run as
part of `cargo test` in CI. A regression in any stable module fails the build.

The second bar exists because the first one alone is not what the tier name
suggests. Under the frontend-only definition all 49 modules passed while 10 of
them could not be imported by a five-line program at all — missing runtime C
symbols and codegen ICEs live entirely past the frontend. That gap is
`AR-F1` in [`alpha-review-findings.md`](alpha-review-findings.md).

Alpha-stable guarantees the module **type-checks against the current language
and is usable from a real program**. It is not a guarantee that every function
in it returns the right answer, and not yet a guarantee of API stability across
versions — that bar is raised post-alpha.

## Alpha-stable modules (49)

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
| Concurrency / systems | `event`, `raft` |

## Experimental modules (excluded)

These modules depend on language features that are not yet implemented and are
**not** part of the alpha-stable subset. They are tracked in the `EXPERIMENTAL`
list in [`tests/std_stable_subset.rs`](../tests/std_stable_subset.rs).

| Module | Reason |
| --- | --- |
| `test` | Uses `try`/`rescue` exception handling, which has no lexer/parser/HIR support yet. |

## Removed modules

| Module | Disposition |
| --- | --- |
| `sqlite` | Removed. The wrapper never type-checked and shipping a broken binding is worse than shipping none. The C surface (`runtime/sqlite.c`) remains and is usable directly via `extern *jinn_sqlite_*` declarations when the toolchain finds `sqlite3`. |

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
