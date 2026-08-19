# Standard library and the stability contract

What ships in `std/`, what "stable" means for each surface of the project, and
how both are enforced.

## Using the standard library

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

Two conventions:

- **Functions are called qualified** by module name: `convert.parse_int(…)`.
- **Types are global and unqualified**: a `TcpStream` from `net` is written
  `TcpStream`, not `net.TcpStream`.

## Stability tiers

Jinn is pre-1.0. During the alpha series the language and standard library are
still moving; the tiers below describe the *intent* and the *enforced*
guarantees today. Full semantic-versioning guarantees begin at `1.0`.

### Stable

A **stable** surface is expected to keep compiling and to keep its observable
behaviour across patch releases within the same minor series.

- **Language** — the core syntax and semantics exercised by `tests/` and by the
  example programs in `apps/` and `benchmarks/`: functions (`*name`),
  `type`/`enum` declarations, `match`, the expression and statement grammar,
  actors and channels, and the `store` surface. The `String` UTF-8 contract is
  pinned separately in [`strings.md`](strings.md).
- **Standard library** — the alpha-stable subset enumerated below.

Stability at the alpha tier guarantees the surface **type-checks against the
current language and is usable from a real program**. API stability across
*minor* versions is a stronger bar, raised at `1.0`.

### Experimental

An **experimental** surface may change or be removed without notice, and is not
covered by the guarantees above. In the language this covers partially
implemented features and anything behind active design; in the standard library
it covers any module not in the alpha-stable subset.

### Deprecated

A **deprecated** surface still works but is scheduled for removal. Deprecations
are announced in the changelog for the release that introduces them and are
removed no earlier than the next minor release. Where practical the compiler
emits a diagnostic pointing at the replacement.

## What "alpha-stable" means for a `std/` module

A module is alpha-stable when it clears **two** bars.

First, it passes the compiler frontend — lex, parse, type-check, HIR lowering —
in library mode, which covers every function in the module including ones no
caller reaches:

```sh
jinnc std/<module>.jn --lib --emit-hir
```

Second, a program that imports it compiles all the way through codegen, links
against the runtime, and runs:

```sh
printf 'use std/<module>\n\n*main\n    log(1)\n    0\n' > importer.jn
jinnc importer.jn -o importer && ./importer
```

Both bars are enforced automatically by `std_stable_subset_frontend_checks` and
`std_stable_subset_imports_and_links` in
[`../tests/std_stable_subset.rs`](../tests/std_stable_subset.rs), which run as
part of `cargo test` in CI. A regression in any stable module fails the build.

The second bar exists because the first alone is not what the tier name
suggests. Missing runtime C symbols, codegen ICEs, and LLVM verification
failures all live entirely past the frontend, and a frontend-only definition
once certified ten modules that a five-line importer could not compile at all.
More generally: **a frontend gate certifies that code type-checks, nothing
more** — `--emit-hir` does not even run codegen, so anything past the frontend
(codegen, missing runtime symbols, the link itself) can still fail.

Alpha-stable is not a guarantee that every function returns the right answer,
and not yet a guarantee of API stability across versions.

## Alpha-stable modules (47)

| Domain | Modules |
| --- | --- |
| Core / language | `convert`, `fmt`, `bytes`, `bit`, `binary`, `volatile`, `collections`, `sort`, `arena` |
| Numerics | `math`, `complex`, `rational`, `decimal`, `bigint`, `stats`, `fft`, `random` |
| Text | `strings`, `regex`, `glob`, `codec`, `hex`, `uuid` |
| Data formats | `json`, `csv`, `toml`, `dataframe` |
| Crypto | `crypto`, `aes`, `argon`, `blake`, `sha`, `tls` |
| I/O and OS | `io`, `fs`, `path`, `os`, `args`, `signal`, `terminal`, `logging` |
| Networking | `net`, `http`, `url` |
| Time | `time`, `date` |
| Concurrency / systems | `event` |

## Provisional modules

These compile and link (the two gates pass) but their observable behavior has
never been meaningfully exercised, and known behavioral defects are on file —
so they carry **no stability promise** until they gain behavioral tests:

| Module | Why provisional |
| --- | --- |
| `raft` | Aspirational. Its inertness was rooted in DIST-1 (mutation through a free-function parameter never reached the caller), closed in [167] — single-node election now converges follower → candidate → leader, pinned in `tests/stdlib/raft_tests.jn`. Multi-node replication, log matching, and commit advancement remain unexercised (roadmap DIST-2/STD-7). |
| `bangle` | Routes 404 in the only end-to-end probe run against it (roadmap STD-10). |
| `process` | `run`/`run_argv` return an empty string: [167] removed their 64 KB truncation, but the replacement reads its length through an extern out-parameter, and those writes are invisible to the caller (roadmap CG-7). |

(`dataframe` and the crypto stack left this list in [167]: dataframe's sort
defect was fixed and both gained behavioral suites in `tests/stdlib/`.)

## Experimental modules

Excluded from the subset because they depend on language features that do not
exist yet. Tracked in the `EXPERIMENTAL` list in
[`../tests/std_stable_subset.rs`](../tests/std_stable_subset.rs).

| Module | Reason |
| --- | --- |
| `test` | Uses `try`/`rescue` exception handling, which has no lexer, parser, or HIR support. |

The gate is self-policing: `std_experimental_list_is_accurate` asserts that
every excluded module *still fails* at least one of the two gates. When the blocking
feature lands and the module compiles, that test fails until the entry is
removed — at which point the module joins the stable subset and is gated like
the rest.

## Removed modules

| Module | Disposition |
| --- | --- |
| `sqlite` | Removed — the wrapper never type-checked, and shipping a broken binding is worse than shipping none. The C surface (`runtime/sqlite.c`) remains and is usable directly via `extern *jinn_sqlite_*` declarations when the toolchain finds `sqlite3`. |

## How stability is enforced

| Surface | Mechanism |
| --- | --- |
| Stable stdlib subset | `tests/std_stable_subset.rs` — frontend gate plus import-and-link gate |
| Stdlib behavior | `tests/stdlib_behavior.rs` — compiles and runs every `tests/stdlib/*_tests.jn` suite (assert-level pins for the high-traffic modules) |
| Experimental accuracy | `std_experimental_list_is_accurate` — excluded modules must still fail |
| `apps/` corpus | `tests/apps_build.rs` — builds and runs all 21 projects |
| Language corpus | `tests/corpus_differential.rs` — compiles **and runs** `snippets/` and `tests/programs/` at `--opt 0` and `--opt 3`, diffing stdout and exit code |
| `String` semantics | `tests/string_unicode.rs` |
| Documented examples | `tests/doc_examples.rs` — every fenced example in `jinn.md` |
| Grammar | `tests/ebnf_roundtrip.rs` — against `docs/jinn.ebnf` |
| Formatting of the compiler itself | `cargo fmt --check`, `cargo clippy --release -- -D warnings` |

## Policy

1. New modules added to `std/` are expected to pass both gates.
2. A module may only be added to `EXPERIMENTAL` with a written justification and
   a tracked language-feature gap.
3. When a language feature lands, the corresponding experimental entry must be
   removed in the same change that makes the module compile.
4. Promoting a module to stable: it must pass both gates; remove its
   `EXPERIMENTAL` entry in the same change.
5. Demoting or removing a stable surface before `1.0` is allowed, but must be
   recorded in the changelog.
