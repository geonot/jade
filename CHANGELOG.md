# Changelog
- **[19]** (2026-06-06 00:40) task 2-4-4: typer quaternary lowering + fallibility. Lower `Expr::Quaternary` (and `!!`-less Ternary over Result/Option) to a `Block` evaluating the subject once then an `EnumIs`-gated ternary; bind `$` to the unwrapped success value (intercept `Placeholder` via `dollar_stack`, suppress placeholder-currying) and `err` to the unwrapped error. `!! err` / bare `!!`-less default propagates via `propagate_err_value` (From-converts `X -> E` into the enclosing Result, R1/R3 diagnostics). Implicit propagation: a bare fallible bind in a fallible-capable fn (`v is f()`) desugars to `f() ? $ !! err`; gated off variant ctors, annotated binds, and non-fallible fns (R4: `main`). Migrate statement/bind `?`/`!`/`!!` parsing to the pratt quaternary (delete legacy `finish_bare_*` + bind handler-chains); fix nested boolean ternary in arms. Add 11 quaternary conformance tests; mark deprecated prefix-`!`-raise corpus `#[ignore]` (task 2-4-7).
- **[18]** (2026-06-06 06:50) task 2-4-3: parser quaternary `e ? ok ! nothing !! err` — add Expr::Quaternary AST node, parse inline + multiline arms (Quaternary iff `!!` present, else Ternary), structural arms in fmt/resolve/scc/implicit/decl, typer stub for 2-4-4, 5 parser tests
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
