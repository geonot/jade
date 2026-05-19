# Jinn LSP Feature Matrix

`jinnc-lsp` is a Language Server Protocol implementation shipped as a
binary alongside `jinnc`. It implements a curated subset of LSP 3.17
sufficient for "good enough" editor support today: completion, hover,
go-to-definition, find-references, rename, document symbols, semantic
tokens, signature help, and diagnostics.

This document is the canonical contract for what the LSP does and does
not do. The smoke-test driver at [tests/lsp_smoke.rs](tests/lsp_smoke.rs)
exercises every entry below; any feature listed as **stable** is also
covered by a passing test in CI.

## Architecture

```
editor  <-- JSON-RPC over stdio -->  jinnc-lsp binary  -->  src/lsp/handlers.rs
                                                            src/lsp/analysis.rs
                                                            (lexer + parser, no codegen)
```

- Transport: stdio, Content-Length framing — see [src/lsp/transport.rs](src/lsp/transport.rs).
- Dispatch: synchronous per-request — see [src/lsp/main.rs](src/lsp/main.rs).
- Analysis: lexer + parser only; the type-checker and codegen are
  intentionally **not** invoked on every keystroke. Symbol detail comes
  from the AST.
- State: in-memory `ServerState { files, workspace_index }` — see
  [src/lsp/handlers.rs](src/lsp/handlers.rs).

## Feature Matrix

| LSP method                       | Status     | Notes                                                                                       |
| -------------------------------- | ---------- | ------------------------------------------------------------------------------------------- |
| `initialize`                     | stable     | Advertises the capabilities below; server version reported as `0.2.0`.                      |
| `textDocument/didOpen`           | stable     | Indexes the file and publishes diagnostics.                                                 |
| `textDocument/didChange`         | stable     | Full-document replace; re-indexes and republishes diagnostics.                              |
| `textDocument/didClose`          | stable     | Drops the file from state.                                                                  |
| `textDocument/hover`             | stable     | Returns Markdown signature for known idents; `null` otherwise.                              |
| `textDocument/definition`        | stable     | Same-file + cross-file via `workspace_index`. Single-location only.                         |
| `textDocument/references`        | stable     | Lexical match across all open files; no semantic shadowing analysis.                        |
| `textDocument/rename`            | stable     | Lexical rename across all open files; no scope analysis (rename of `x` renames every `x`).  |
| `textDocument/documentSymbol`    | stable     | Top-level fns, types (+ fields, methods), enums (+ variants), constants, externs.           |
| `textDocument/completion`        | stable     | Keyword list + workspace symbols. No context-aware filtering yet.                           |
| `textDocument/semanticTokens/full` | stable   | Token legend: keyword, function, variable, string, number, operator, type, comment, enumMember. |
| `textDocument/signatureHelp`     | stable     | Active-parameter detection by comma counting inside the current call site.                  |
| `textDocument/codeAction`        | **not impl** | Planned post-alpha.                                                                       |
| `textDocument/formatting`        | **not impl** | Use `jinnc fmt` for now.                                                                  |
| `workspace/symbol`               | **not impl** | Per-file symbols only.                                                                    |
| `textDocument/inlayHint`         | **not impl** |                                                                                            |
| `textDocument/foldingRange`      | **not impl** |                                                                                            |
| `textDocument/typeDefinition`    | **not impl** |                                                                                            |
| `textDocument/implementation`    | **not impl** |                                                                                            |

## Known limitations

- **No type-aware analysis.** Hover signatures, definitions and renames
  use only the parser. Identifiers that shadow each other are treated
  as one.
- **Diagnostics are parse-level only.** Type errors do **not** appear in
  the editor; run `jinnc <file>` to see them.
- **Single-file indexing per request.** `workspace_index` populates as
  files are opened, not eagerly across the project tree.
- **No incremental sync.** Every `didChange` payload is treated as the
  full document text.

## Smoke-test driver

`cargo test --test lsp_smoke` exercises the matrix above. The tests call
`src/lsp/handlers.rs` entry points directly with synthesised JSON
params; no subprocess or stdio plumbing is involved. This keeps the
matrix in CI on every PR.

To add coverage for a new capability:

1. Implement the handler in `src/lsp/handlers.rs` and wire it in
   `src/lsp/main.rs`.
2. Add a row to the matrix above with status `stable`.
3. Add a corresponding `#[test]` in [tests/lsp_smoke.rs](tests/lsp_smoke.rs).

## Verbose logging

The server inherits the `tracing` setup added in B.3 (P1-6). Set
`JINN_LOG=jinnc::lsp=debug` to see request/response traffic on stderr.
