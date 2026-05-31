# Jinn Language for VS Code

Syntax highlighting and language server support for the [Jinn programming language](https://github.com/your-org/jinn).

## Features

- Syntax highlighting for all Jinn constructs
- Proper indentation rules for block-based syntax
- Comment toggling with `#`
- Auto-closing pairs for brackets and strings
- Off-side folding (indentation-based)

### Language server (jinnc-lsp)

When the `jinnc-lsp` server is available, the extension also provides:

- Live diagnostics (errors/warnings as you type)
- Hover information
- Go to definition
- Document symbols (outline / breadcrumbs)
- Completion (triggered on `.`)
- Find all references
- Rename symbol
- Semantic token highlighting
- Signature help (triggered on `(` and `,`)

The server is built from this repository as the `jinnc-lsp` binary
(`cargo build --release` produces `target/release/jinnc-lsp`). The extension
finds it automatically when the repository is open as a workspace folder; you
can also point at a specific binary with the `jinn.lsp.serverPath` setting, or
disable the server entirely with `jinn.lsp.enable`. Syntax highlighting works
regardless of whether the server is present.

#### Settings

| Setting | Default | Description |
| ------- | ------- | ----------- |
| `jinn.lsp.enable` | `true` | Enable the language server. |
| `jinn.lsp.serverPath` | `""` | Path to `jinnc-lsp` (absolute or workspace-relative). Empty = auto-detect (`target/release` → `target/debug` → `PATH`). |

The **Jinn: Restart Language Server** command restarts the server.

## Supported Constructs

- Function definitions (`*name(...)`)
- Type and enum definitions
- Control flow: `if`/`elif`/`else`, `while`, `for`/`in`/`to`/`by`, `loop`, `match`
- Pattern matching with `?` arms
- Pipeline operator `~`
- Lambda expressions (`|params| body`)
- Bindings with `is`
- Built-in types: `i8`–`u64`, `f32`, `f64`, `bool`, `str`, `void`
- Literals: integers (decimal, hex, binary, octal), floats, strings, raw strings
- Placeholder `$` / `$N`

## Installation

### From Source

```bash
# Install vsce if not already installed
npm install -g @vscode/vsce

# Install dependencies and compile the language client
cd vscode-jinn
npm install
npm run compile

# Package the extension
vsce package

# Install the .vsix file
code --install-extension jinn-lang-0.2.0.vsix
```

Build the language server too, so the extension can find it:

```bash
cargo build --release   # produces target/release/jinnc-lsp
```

### Development

1. Open this folder in VS Code
2. Run `npm install` then `npm run compile` (or `npm run watch`)
3. Press `F5` to launch the Extension Development Host
4. Open any `.jn` file to see syntax highlighting and language features
