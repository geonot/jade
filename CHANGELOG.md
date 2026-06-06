# Changelog
- **[15]** (2026-06-06 05:34) task 2-4-2: remove ?> propagation; err-raise + !! lex/parse intact
- **[11]** (2026-06-04 12:45) Implement Option/Result prelude combinator surfaces (task 2-2)

Add full combinator method surfaces shared by typer+codegen for the canonical
prelude Option of T and Result of T,E per docs/error-effects.md §2:
Option{is_some,is_none,unwrap,unwrap_or,map,and_then,ok_or},
Result{is_ok,is_err,unwrap,unwrap_or,map,map_err,and_then,ok,err}.

Combinators desugar to EnumIs/EnumUnwrap/VariantCtor/IndirectCall via Ternary,
monomorphizing target enums through the existing enum mono scheme. Extend the
type-annotation parser to accept multi-arg generics (Result of T, E) in
returns/bind positions via a context flag, avoiding the param-list comma
ambiguity. Add tests/error_effects.rs conformance suite.
- **[9]** (2026-06-04 12:34) docs: checked error-effect system design spec (task 2-1)
- **[8]** (2026-06-04 12:30) Pin String UTF-8 semantics: scalar .length, byte .byte_count, spec + conformance tests; fix raw-string lexer double-encoding
- **[6]** (2026-06-04 12:15) Shared builtin-method registry: single source of truth for typer+codegen (Vec/Map/String); fix chain/flatten/enumerate/keys/values drift
- **[3]** (2026-06-04 11:56) traits: synthesize default trait method bodies into impls; add tests/traits.rs conformance suite
- **[1]** (2026-06-04 11:50) P0(1-1): clippy hard gate in CI (-D warnings), fix all 784 clippy warnings + approx_constant error, clean store/wal artifacts from tree
