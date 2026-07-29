# §3 Architecture — the Jinn pipeline at a glance

```
                                                  ┌──────────────────────────────────────────────────┐
                                                  │           jinnc (Rust, 84 kLOC)                  │
                                                  └──────────────────────────────────────────────────┘

  ┌────────┐ tokens  ┌────────┐ AST  ┌─────────┐ Type+HIR  ┌────────────┐ HIR  ┌────────┐ MIR   ┌──────────┐ MIR'  ┌──────────┐  obj   ┌────────┐  exe
  │ Lexer  │────────▶│ Parser │─────▶│  Typer  │──────────▶│ HirValidate│─────▶│  MIR   │──────▶│ Perceus  │──────▶│  LLVM    │───────▶│ Linker │──────▶  ./app
  │ 1.4kLOC│         │ 6.1k   │      │ 15.9k   │           │   plus     │      │  6.4k  │       │  0.9k    │       │ codegen  │        │ (ld /  │
  └────────┘         └────────┘      └─────────┘           │ Ownership  │      └────────┘       └──────────┘       │  29.7k   │        │  lld)  │
        │                  │              │                │   1.0k     │                                          └──────────┘        └────────┘
        │                  │              │                └────────────┘                                                │
        │                  │              │                                                                              │
        │                  │              │                                                                              ▼
        │                  │              │                                                                       ┌──────────────┐
        │                  │              │                                                                       │ libjinn_rt.a │
        │                  │              │                                                                       │ (C, 6.3 k    │
        │                  │              │                                                                       │  LOC)        │
        │                  │              │                                                                       │  + libc,     │
        │                  │              │                                                                       │  + sqlite,   │
        │                  │              │                                                                       │  + openssl?  │
        │                  │              │                                                                       └──────────────┘
        │                  │              │
        ▼                  ▼              ▼
  errors.rs           pre/post-stmt   monomorph
  span+line info     splice queue    + escape tiers
                                      (T1/T2/T3)
```

## Major design choices that shape the rest of the report

1. **Significant indentation, `#` comments, no semicolons.** The lexer
   is indent-aware. Indent uses spaces only — `\t` is rejected at the
   lexer. A *very large* keyword table (≈ 80 reserved words) is the
   single source of truth (`src/lexer/mod.rs:36`).

2. **Hand-rolled recursive-descent parser** (`src/parser/`) with:
   - A `binop!` macro for left-associative chains.
   - A `pending_pre_stmts` / `pending_post_stmts` queue so a single
     surface statement (e.g. the `a is x() ! Variant` early-return
     sugar) can desugar into a sequence and have the extra statements
     spliced into the enclosing block.
   - A `label_stack` so `break LABEL` / `continue LABEL` can recognise
     labels rather than treat them as identifiers.
   - A 20-error cap; further parse errors are dropped.

3. **One mega-pass typer** that does **name resolution, type inference,
   ownership classification, escape analysis, generic
   monomorphisation, and HIR lowering all in one pass.** This is
   discussed at length in §7; it is the single biggest architectural
   risk in the project. The `Typer` struct has ≈ 50 fields.

4. **Two parallel codegen paths.** This is not advertised anywhere in
   the source, but it is real:
   - A "HIR-direct" path in `src/codegen/{arith.rs, expr/, stmt/, …}`
     used for some constructs.
   - A "MIR-based" path in `src/codegen/mir_codegen/` used for the
     primary IR.
   The two have drifted. **`checked_divmod` exists in the HIR path
   and is wired up; the MIR path bypasses it.** That is the root cause
   of the integer division-by-zero UB documented in §11/§21.

5. **MIR is SSA with a side-table.** Drop instructions are emitted
   explicitly into the IR; the Perceus side-table (`PerceusMeta`) only
   carries metadata that codegen cannot recover from the IR — reuse
   slots, drop-fusion runs, tail reuse, pool hints, Vec-slot semantics
   (`src/mir/mod.rs:42`).

6. **Perceus is implemented at the MIR level.** The older HIR-level
   Perceus is retired and only the MIR-level pass (`src/perceus/`)
   runs. It performs: drop elision for trivials, drop sinking, drop
   fusion, reuse pairing, FBIP hinting, tail reuse, pool-allocation
   hints, and Vec-slot recognition.

7. **Runtime is C, statically linked.** The runtime
   (`runtime/`, 6.3 k LOC) provides stackful coroutines with arch-
   specific context switch (`context_x86_64.S`, `context_aarch64.S`),
   an M:N work-stealing scheduler, typed channels, actor mailboxes,
   `select`, timers, a WAL + multiple persistent stores
   (bloom/column/fts/index/kv/vec/sqlite/vec/wal), crypto, fs, net,
   tls, terminal, fts, migrate. **It is `unsafe` from end to end and
   has zero TODO/FIXME markers, which means whatever issues exist are
   not labelled.**

8. **Stdlib is two-tier:**
   - `libjn/` (3.1 k LOC) — libc-shaped, low-level (stdio.jn,
     string.jn, stdlib.jn, etc.). One module per common libc header.
   - `std/` (11.8 k LOC) — high-level, idiomatic. No public stability
     contract is documented.

9. **The driver** (`src/driver/`, 3.1 k LOC) handles CLI, project
   layout, multi-file compilation, and incremental compilation hints.
   Source loading is in `src/driver/sources/`. The CLI exposes
   `--target`, `--cpu`, `--features`, `--standalone`, `--debug-perceus`,
   and a number of dev flags. There is no package manager in the
   classical Cargo sense; multi-file projects are organised by
   filesystem layout.

10. **LSP shipping in-tree.** `jinnc-lsp` is built alongside `jinnc`,
    and there is a VS Code extension under `vscode-jinn/` plus a
    tree-sitter grammar under `tree-sitter-jinn/`. Each is reviewed
    in §17.

## Where bugs live

Cross-referencing the above with §21:

| Bug                                | Resp. layer                           |
| ---------------------------------- | ------------------------------------- |
| Int div/mod-by-zero UB             | MIR-codegen path; mismatch with HIR-codegen `checked_divmod` |
| Vec OOB SIGSEGV (some paths)       | Codegen — bounds check only on certain access paths (see §11) |
| Generator emits invalid LLVM IR    | MIR→LLVM lowering of coroutines or `yield` return shape |
| `map(v, $ * 2)` ICE                | `helpers/values.rs:96` — `into_int_value()` on a pointer from a closure body |
| Bare top-level expr accepted       | Parser accepts free expression as decl |
| Missing `main` reaches linker      | Driver does not check for `main` symbol before emit |
| `$ * 2` over `[i64]` runtime crash | Closure inference loses element type |

These are not separate bugs — they are symptoms of a single common
cause: **the type/IR contract between layers is not tight enough, and
codegen is silently patching the gaps.** Fixing them is mostly an
exercise in pushing invariants up to the typer and adding asserts in
the receiving layers. See §24 for the plan.
