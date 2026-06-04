# Changelog
- **[8]** (2026-06-04 12:30) Pin String UTF-8 semantics: scalar .length, byte .byte_count, spec + conformance tests; fix raw-string lexer double-encoding
- **[6]** (2026-06-04 12:15) Shared builtin-method registry: single source of truth for typer+codegen (Vec/Map/String); fix chain/flatten/enumerate/keys/values drift
- **[3]** (2026-06-04 11:56) traits: synthesize default trait method bodies into impls; add tests/traits.rs conformance suite
- **[1]** (2026-06-04 11:50) P0(1-1): clippy hard gate in CI (-D warnings), fix all 784 clippy warnings + approx_constant error, clean store/wal artifacts from tree
