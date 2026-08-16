# Jinn documentation

Jinn is a compiled, statically typed language with indentation syntax, word
operators, ownership-based memory management, and no garbage collector. This
directory is the whole documentation set.

Docs describe the language **as implemented**. Where the implementation falls
short of a contract, the gap is stated inline and carries an id into
[`roadmap.md`](roadmap.md) — the single list of open work. There is no separate
review, audit, or remediation tracking.

## Using the language

| Document | What it is |
| --- | --- |
| [`jinn.md`](jinn.md) | **Start here.** The language tour, from `log('hello')` to stores and concurrency, plus the idiomatic-style guide and the reserved-word list. Every example in it is compiled by CI. |
| [`stdlib.md`](stdlib.md) | What ships in `std/`, the stability tiers, and how membership in each is machine-checked. |
| [`tooling.md`](tooling.md) | The `jinnc` CLI and its flags, subcommands, the formatter's current state, `jinn bind`, the language server, and environment variables. |

## Specifications

Normative. The implementation is wrong wherever it disagrees with one of these,
and each is pinned by a conformance suite.

| Document | Covers |
| --- | --- |
| [`memory-model.md`](memory-model.md) | Ownership: type categories, moves, inferred borrows, consuming parameters, drop discipline, access modifiers, `@resource`, cross-thread rules. |
| [`concurrency.md`](concurrency.md) | Tasks, channels, actors, `together` scopes, cancellation, error propagation from children, and — the part that is easiest to get wrong — shutdown. |
| [`error-effects.md`](error-effects.md) | Errors as values: the `Option`/`Result` prelude, `err` declarations and raises, the quaternary, implicit propagation with inferred fallibility, `From` conversion. |
| [`strings.md`](strings.md) | The `String` UTF-8 contract and the deliberate split between the scalar view and the byte view. |
| [`jinn.ebnf`](jinn.ebnf) | The grammar. |

## Working on the compiler

| Document | Covers |
| --- | --- |
| [`internals.md`](internals.md) | Building, the compilation pipeline, the source trees, the incremental-compilation post-mortem, how to write tests that scale, the gates, and project conventions. |
| [`roadmap.md`](roadmap.md) | Every known open item, and where the language stands relative to alpha. |
| [`SECURITY.md`](SECURITY.md) | Reporting a vulnerability; what is and is not in scope. |

## Design — specified ahead of implementation

Design documents are written before the code. Several have since shipped in
large part — `freeze`, views, closure captures, the formatter's trivia step,
and the capabilities pass are all built and conformance-gated — and each
document's status header states exactly which steps are real and which
remain.

| Document | Covers |
| --- | --- |
| [`design/lamp.md`](design/lamp.md) | The package manager, registry protocol, `.jnb` binary library format, supply-chain trust layer, and deployments. |
| [`design/compiler-prereqs.md`](design/compiler-prereqs.md) | The compiler machinery lamp assumes: capabilities as an effect pass (shipped), path-scoped package identity, interface v2 with the three-hash ladder, and config blocks (the rest unbuilt). |
| [`design/fmt-and-lint.md`](design/fmt-and-lint.md) | The formatter and linter: lossless syntax tree, tiered rule engine, and the verifier that makes behaviour preservation checkable rather than hoped for. |
| [`design/libjn.md`](design/libjn.md) | A C standard library in Jinn, and the language gaps it is a forcing function for. |
| [`design/second-class-refs.md`](design/second-class-refs.md) | Borrowed views in parameter, expression, and yield positions — never storable, so no lifetime syntax; the honest fix for element-read deep copies. |
| [`design/freeze.md`](design/freeze.md) | One-way deep immutability for sharing read-only data across tasks without copies or refcounts. |
| [`design/closure-captures.md`](design/closure-captures.md) | Capture rules for closures and generators — implemented through step 2; the status header carries the residue. |
