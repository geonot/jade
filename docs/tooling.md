# Toolchain

`jinnc` is the compiler. `jinn` is the same binary under a friendlier name;
`jinnc-lsp` is the language server. Everything below describes what the tools do
today; known gaps are `X-1` and `X-3`–`X-5` in [`roadmap.md`](roadmap.md#tooling).

## Compiling

```sh
jinnc file.jn -o prog        # compile and link
jinnc file.jn --lib          # library mode: check every function, no entry point
jinnc file.jn --emit-hir     # also --emit-mir, --emit-llvm, --emit-obj
jinnc file.jn --opt 0        # 0..3, default 3
```

| Flag group | Flags |
| --- | --- |
| Output | `-o`, `--emit-llvm`, `--emit-hir`, `--emit-mir`, `--emit-obj`, `--emit-interface` |
| Codegen | `--opt N`, `--lto`, `--lib`, `--link`, `--standalone`, `--threads N` |
| Target | `--target`, `--cpu`, `--features` |
| Floating point | `--fast-math`, `--deterministic-fp` |
| Type checking | `--strict-types`, `--lenient`, `--pedantic`, `--warn-inferred-defaults` |
| Debugging | `--debug`, `--debug-types`, `--debug-drops`, `--dump-tokens`, `--dump-ast`, `-v` |

`--target`/`--cpu`/`--features`, `--lto`, `--fast-math`, `--deterministic-fp`
and `--standalone` are untested surfaces (`V-2`); `--opt 2` has only been
spot-checked.

### Subcommands

| Command | Purpose |
| --- | --- |
| `run` | Compile and run, with a hash-keyed binary cache. Dependency edits invalidate the cache. |
| `test` | Run a program's tests. |
| `check` | Type-check without emitting an object. Runs the same ownership analysis as `build`. |
| `fmt` | Format source (prints to stdout; `--write` rewrites in place). |
| `init` | Scaffold a project (`project.jn` + `source/`). |
| `bind` | Generate Jinn `extern` declarations from a C header. |
| `fetch`, `update`, `build`, `package`, `publish` | Package commands. Only `fetch` has been exercised (`V-3`). |

## What `--emit-hir` certifies

`--emit-hir` and `--emit-mir` skip the linker, which makes them roughly four
times faster than a full compile and the right tool for frontend-only
assertions. They are **not** a proxy for "this program is correct": codegen
never runs, and some failures — codegen ICEs, missing runtime symbols, the
link itself — live entirely past the frontend. Treat a green `--emit-hir` as
"it type-checks", nothing more.

## `jinn fmt`

`jinn fmt` lexes, parses to an AST, and pretty-prints it back to source.

Formatting is idempotent, preserves comments, and is held to the grammar by a
CI gate: every file in `snippets/`, `tests/programs/`, `benchmarks/`, and
`std/` must still frontend-check after formatting
(`fmt_output_still_frontend_checks_over_corpus`, [144]). As a second line of
defense, `--write` re-parses its own output before touching the file and
refuses — with a formatter-bug message and a non-zero exit — if the result no
longer parses. Multi-module `apps/` projects are not yet in the gate, and a
few expression forms that never appear in expression position in the corpus
still print as placeholders; the residue is tracked as `X-1`.

The intended end state — a behaviour-preserving idiomatic rewriter and linter —
is specified in [`design/fmt-and-lint.md`](design/fmt-and-lint.md). The style it
targets is documented in [`jinn.md`](jinn.md#idiomatic-jinn).

## `jinn bind`

`jinn bind <header.h>` generates Jinn `extern *` declarations from a C header.
It emits `#` comments, drops multi-line macros, keeps block comments out of
signatures, and skips what it cannot represent with a stated reason. Verified
against `errno.h`, `string.h`, and `stdlib.h`: 97 externs generated, all
parseable.

It is a textual approximation of C, not a C parser, and the generated file says
so at the top. Review the output before depending on it.

## Language server

`jinnc-lsp` speaks LSP 3.17 over stdio with Content-Length framing.

```
editor  <-- JSON-RPC over stdio -->  jinnc-lsp  -->  src/lsp/handlers.rs
                                                     src/lsp/analysis.rs   (lexer + parser)
                                                     src/lsp/typed.rs      (full frontend)
```

Dispatch is synchronous per request; state is an in-memory
`ServerState { files, workspace_index, typed }`. Every `didOpen`/`didChange`
runs the **full frontend** (lex, parse, module resolution, type inference — no
codegen) on a dedicated 256 MiB-stack thread with panic containment, so a
compiler bug degrades to one "internal error" diagnostic instead of killing
the server. The result feeds diagnostics, hover, definition, and rename; the
parse-level analyzer remains as the fallback whenever typing fails.

| Method | Status | Notes |
| --- | --- | --- |
| `initialize` | stable | Advertises the capabilities below. |
| `textDocument/didOpen` | stable | Runs the frontend; publishes lex, parse, and **type** diagnostics with real positions, warnings included. |
| `textDocument/didChange` | stable | Full-document replace; re-checks and republishes. |
| `textDocument/didClose` | stable | Drops the file from state. |
| `textDocument/hover` | stable | Inferred types for locals, params, fields, and binds; full signatures for functions. |
| `textDocument/definition` | stable | Resolved through the typer's `DefId`s, so shadowed identifiers resolve to the right binder. Cross-file falls back to the lexical index. |
| `textDocument/references` | stable | Lexical match across open files. |
| `textDocument/rename` | stable | Scope-aware within the file (renames exactly the occurrences of the resolved binder); falls back to lexical rename across open files when typing fails. |
| `textDocument/documentSymbol` | stable | Top-level functions, types, enums, constants, externs. |
| `textDocument/completion` | stable | Keywords plus workspace symbols; no context filtering. |
| `textDocument/semanticTokens/full` | stable | keyword, function, variable, string, number, operator, type, comment, enumMember. |
| `textDocument/signatureHelp` | stable | Active parameter by comma counting. |
| `codeAction`, `formatting`, `workspace/symbol`, `inlayHint`, `foldingRange`, `typeDefinition`, `implementation` | not implemented | |

Positions convert between UTF-16 code units and byte offsets in both
directions, so hover and go-to-definition keep working to the right of an emoji
or an accented character.

**Known limitations** (`X-5`): package imports are not resolved in the editor
(module resolution runs with an empty package set; a file whose imports cannot
resolve degrades to parse-level analysis with a warning saying so);
cross-file rename and references are still lexical; generic functions lose
hover/rename at instantiation sites (monomorphized names do not match source
spelling); the workspace index populates as files are opened rather than
eagerly; every `didChange` is treated as the full document text.

`cargo test --test lsp_smoke` exercises every row above by calling the handler
entry points directly with synthesised JSON params — no subprocess, no stdio
plumbing. To add a capability: implement the handler, wire it in
`src/lsp/main.rs`, add a row here with status `stable`, and add a `#[test]`.

## Editor support

- **VS Code** — `vscode-jinn/` (client for `jinnc-lsp`, syntax highlighting).
- **Tree-sitter** — `tree-sitter-jinn/` (grammar for editors that consume one).

## Environment variables

| Variable | Effect |
| --- | --- |
| `JINN_RT_DEBUG=1` | Build the C runtime `-O0 -g`. |
| `JINN_MIR_VERIFY=0` | Opt out of MIR verification (on by default, including in release). |
| `JINN_PROPTEST_CASES=N` | Property-test case count. |
| `JINN_WAL_SYNC` | Testing override for the WAL sync policy; when set it overrides every store's `@durable`/`@relaxed`/`@volatile` decorator. `group` batches syncs at transaction commits and falls back to per-record `fdatasync` outside them. |
| `JINN_LOG` | `tracing` filter, e.g. `jinnc::lsp=debug`. |
| `LLVM_SYS_211_PREFIX` | Required for a *fresh* compiler build on Arch: `/usr/lib/llvm21`. |
