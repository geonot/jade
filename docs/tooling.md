# Toolchain

`jinnc` is the compiler. `jinn` is the same binary under a friendlier name;
`jinnc-lsp` is the language server. Everything below describes what the tools do
today; known gaps are `X-1`–`X-5` in [`roadmap.md`](roadmap.md#tooling).

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
| `fmt` | Format source — **read the warning below**. |
| `init` | Scaffold a project (`project.jn` + `source/`). |
| `bind` | Generate Jinn `extern` declarations from a C header. |
| `fetch`, `update`, `build`, `package`, `publish` | Package commands. Only `fetch` has been exercised (`V-3`). |

## What `--emit-hir` certifies

`--emit-hir` and `--emit-mir` skip the linker, which makes them roughly four
times faster than a full compile and the right tool for frontend-only
assertions. They are **not** a proxy for "this program is correct": codegen
never runs, and some errors are still only caught there. A program using an
undefined variable compiles to HIR with exit 0 and fails at codegen with no span
(`T-3`). Treat a green `--emit-hir` as "it type-checks", nothing more.

## `jinn fmt`

`jinn fmt` lexes, parses to an AST, and pretty-prints it back to source.

> **Do not use `--write` on code you care about.** Sweeping the 688-file corpus
> through `fmt` leaves **164 files that no longer compile** — including 41 of
> the 50 modules in `std/`. `--write` applies this silently and exits 0. The
> damage is printer grammar drift: store, extern, actor and query forms, the
> `...` placeholder, and parenthesization precedence. Tracked as `X-1`.

String literals escape correctly and formatting is idempotent; the printer's
coverage of the rest of the grammar is what fails. The existing gate checks
idempotence and comment preservation but never that the output still compiles,
which is why this went unnoticed. Until `X-1` closes, use `jinn fmt` to read
formatted output, not to rewrite files.

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
                                                     src/lsp/analysis.rs
                                                     (lexer + parser, no codegen)
```

Dispatch is synchronous per request; state is an in-memory
`ServerState { files, workspace_index }`. Analysis is **lexer and parser only** —
the type-checker and codegen are deliberately not invoked on every keystroke.

| Method | Status | Notes |
| --- | --- | --- |
| `initialize` | stable | Advertises the capabilities below. |
| `textDocument/didOpen` | stable | Indexes the file and publishes diagnostics. |
| `textDocument/didChange` | stable | Full-document replace; re-indexes and republishes. |
| `textDocument/didClose` | stable | Drops the file from state. |
| `textDocument/hover` | stable | Markdown signature for known identifiers. |
| `textDocument/definition` | stable | Same-file and cross-file; single location only. |
| `textDocument/references` | stable | Lexical match across open files. |
| `textDocument/rename` | stable | Lexical rename across open files. |
| `textDocument/documentSymbol` | stable | Top-level functions, types, enums, constants, externs. |
| `textDocument/completion` | stable | Keywords plus workspace symbols; no context filtering. |
| `textDocument/semanticTokens/full` | stable | keyword, function, variable, string, number, operator, type, comment, enumMember. |
| `textDocument/signatureHelp` | stable | Active parameter by comma counting. |
| `codeAction`, `formatting`, `workspace/symbol`, `inlayHint`, `foldingRange`, `typeDefinition`, `implementation` | not implemented | |

Positions convert between UTF-16 code units and byte offsets in both
directions, so hover and go-to-definition keep working to the right of an emoji
or an accented character.

**Known limitations** (`X-3`): no type-aware analysis, so identifiers that
shadow each other are treated as one and rename is a lexical find-and-replace;
diagnostics are parse-level only, so type errors never reach the editor; the
workspace index populates as files are opened rather than eagerly; every
`didChange` is treated as the full document text.

`cargo test --test lsp_smoke` exercises every row above by calling the handler
entry points directly with synthesised JSON params — no subprocess, no stdio
plumbing. To add a capability: implement the handler, wire it in
`src/lsp/main.rs`, add a row here with status `stable`, and add a `#[test]`.

Set `JINN_LOG=jinnc::lsp=debug` to see request and response traffic on stderr.

## Editor support

- **VS Code** — `vscode-jinn/` (client for `jinnc-lsp`, syntax highlighting).
- **Tree-sitter** — `tree-sitter-jinn/` (grammar for editors that consume one).

## Environment variables

| Variable | Effect |
| --- | --- |
| `JINN_RT_DEBUG=1` | Build the C runtime `-O0 -g`. |
| `JINN_MIR_VERIFY=0` | Opt out of MIR verification (on by default, including in release). |
| `JINN_PROPTEST_CASES=N` | Property-test case count. |
| `JINN_WAL_SYNC` | Store WAL sync policy, chosen once at first open (`S-5`, `S-6`). |
| `JINN_LOG` | `tracing` filter, e.g. `jinnc::lsp=debug`. |
| `LLVM_SYS_211_PREFIX` | Required for a *fresh* compiler build on Arch: `/usr/lib/llvm21`. |
