# Jinn Language for VS Code

Syntax highlighting and language support for the [Jinn programming language](https://github.com/your-org/jinn).

## Features

- Syntax highlighting for all Jinn constructs
- Proper indentation rules for block-based syntax
- Comment toggling with `#`
- Auto-closing pairs for brackets and strings
- Off-side folding (indentation-based)

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

# Package the extension
cd vscode-jinn
vsce package

# Install the .vsix file
code --install-extension jinn-lang-0.1.0.vsix
```

### Development

1. Open this folder in VS Code
2. Press `F5` to launch the Extension Development Host
3. Open any `.jn` file to see syntax highlighting
