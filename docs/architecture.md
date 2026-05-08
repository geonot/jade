# Jinn Architecture

A single-page tour of the Jinn compiler pipeline. Updated whenever the
pipeline shape changes.

## Pipeline overview

```
   .jn source
       │
       ▼
  ┌──────────┐
  │  Lexer   │  src/lexer.rs               → Vec<Token>
  └──────────┘
       │
       ▼
  ┌──────────┐
  │  Parser  │  src/parser/                → ast::Program
  └──────────┘
       │
       ▼
  ┌──────────┐
  │  Typer   │  src/typer/                 → hir::Program
  └──────────┘   (HM-style inference; ownership/borrow check in src/ownership.rs)
       │
       ▼
  ┌──────────┐
  │ Perceus  │  src/perceus/               → hir::Program
  └──────────┘   (reference-counting insertion)
       │
       ▼
  ┌──────────┐
  │  MIR     │  src/mir/lower.rs           → mir::Program
  │  Lower   │
  └──────────┘
       │
       ▼
  ┌──────────┐
  │ MIR Opt  │  src/mir/opt.rs             → mir::Program
  └──────────┘
       │
       ▼
  ┌──────────┐
  │ Codegen  │  src/codegen/mir_codegen/   → LLVM Module
  └──────────┘
       │
       ▼
  ┌──────────┐
  │   LLVM   │  inkwell wrapper            → object file
  └──────────┘
       │
       ▼
  ┌──────────┐
  │  Linker  │  cc                         → executable
  └──────────┘
```

## Crate layout

| Path | Role |
| --- | --- |
| `src/lexer.rs` | Source → tokens. KEYWORDS table is the source of truth (see [docs/lexer/keywords.md](lexer/keywords.md)). |
| `src/parser/` | Tokens → AST. One file per node family (`expr.rs`, `stmt.rs`, `decl.rs`). |
| `src/ast.rs` | AST data types. |
| `src/typer/` | AST → HIR with type inference, name resolution, ownership analysis. |
| `src/hir.rs` | HIR data types. |
| `src/perceus/` | RC insertion. Two implementations exist; see R11 for the active path. |
| `src/mir/` | HIR → MIR; MIR optimizations. |
| `src/codegen/mir_codegen/` | MIR → LLVM IR. The current production codegen. |
| `src/codegen/*.rs` | Legacy HIR-era codegen helpers; being folded into `mir_codegen` (see CLEANUP.md §C.1). |
| `runtime/` | C runtime: scheduler, channels, actors, store, IO, crypto. ABI documented in `runtime/jinn_rt.h`. |
| `std/` | Standard library, written in Jinn. |
| `src/lsp/` | Language server. |
| `src/main.rs` | Driver / CLI entry point. |
| `src/lib.rs` | Library facade. |

## Runtime ABI

All runtime symbols are declared in `runtime/jinn_rt.h`. Every `.c` file
includes that single header. Conventions:

- **Prefixes**: `jinn_*` for public C API; `c_*` for non-jinn C helpers; `__jinn_*` for codegen-internal symbols.
- **Opaque types**: forward-declared `typedef struct Foo Foo;` in the header; defining `struct Foo { ... };` in the owning `.c` file.
- **Errors**: resource-creating functions return pointer/NULL; mutating functions return `int` errno-style.

See [runtime/README.md](../runtime/README.md) for per-module mapping.

## Where to start (by task)

- **Add a keyword**: edit `src/lexer.rs` KEYWORDS, regenerate `docs/lexer/keywords.md`, wire to a parser arm.
- **Add a builtin**: extend `hir::BuiltinFn`, lower in `src/mir/lower.rs`, codegen in `src/codegen/builtins.rs`.
- **Add a runtime function**: declare in `runtime/jinn_rt.h`, define in the relevant `.c` file, expose to codegen via `src/runtime_ffi.rs`. See `runtime/README.md` step-by-step.
- **Add a stdlib module**: drop `std/foo.jn`, add tests in `tests/programs/std/foo.jn`.

## Cross-cutting docs

- [CLEANUP.md](../CLEANUP.md) — ongoing refactor program.
- [VISION.md](../VISION.md) — design north star.
- [ROADMAP.md](../ROADMAP.md) — release planning.
- [the-way-of-jinn.md](../the-way-of-jinn.md) — language design philosophy.
