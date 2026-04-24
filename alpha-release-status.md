# Alpha Release Readiness Status

Date: 2026-04-24

This document tracks practical verification work completed for alpha-readiness priorities.

## Completed Work

### 1. Multi-file / multi-module projects + stdlib includes

Status: PASS

- Added moderate-complexity project: `examples/alpha_release_demo`
- Uses nested modules:
  - `source/analytics.jade`
  - `source/workers/ingest.jade`
  - `source/reports/reporter.jade`
- Uses stdlib include: `use std/time`
- Exercises module-qualified calls across files.

### 2. Package ecosystem (include + publish via Jade tool)

Status: PASS (local end-to-end workflow)

- Added CLI subcommand: `jade package`
  - Emits `jade.pkg`
  - Emits source archive in `dist/` by default
- Added CLI subcommand: `jade publish`
  - Creates git tag `v<version>` from `project.jade` version
  - Optional push support
  - Prints consumer `require` snippet
- Added `project.jade` metadata parser in package module for reliable dependency handling.
- Fixed transitive dependency parsing in cache resolver to read `project.jade` (not `jade.pkg`).
- Added opt-in `file://` dependency URL support under `JADE_ALLOW_NON_HTTPS_DEPS=1` for local package testing.

### 3. Jade binary command coverage

Status: PASS

- Added `jade` binary alias (in addition to `jadec`).
- Verified command path coverage in smoke checks:
  - `test`, `build`, `run` (via built artifact), `check` (available), `fetch`, `update`, `package`, `publish`, `bind`, `fmt`.

### 4. Actor model + persistent store reliability checks

Status: PASS (functional), PASS (quick sanity perf)

- Moderate demo includes actor + loop handler + store insert/aggregate path.
- Existing actor suite remains green.
- Alpha smoke runs focused benchmark sample:
  - `actor_single`, `actor_pingpong`, `store_ops`

### 5. Cross-compilation / target arch support

Status: PASS (compiler/linker command path), ENV-DEPENDENT (toolchain availability)

- `build` subcommand now accepts and forwards:
  - `--target`, `--cpu`, `--features`, `--standalone`
- Smoke script attempts wasm target build:
  - marks pass if toolchain exists
  - marks skipped if unavailable in current environment

### 6. Comprehensive language guide on website

Status: PASS

- Added `website/guide.html` covering:
  - syntax, modules, CLI, packaging/publish workflow,
  - actors, store, testing, cross compilation, alpha checklist.
- Linked guide from landing page nav.
- Updated landing page actor snippet to method-call messaging syntax.

### 7. Moderate complexity test project

Status: PASS

- Added `examples/alpha_release_demo` with:
  - inline tests (`jade test`)
  - multi-module imports
  - stdlib include
  - actor loop + messaging
  - persistent store workflow

## Automation

Added one-command verification script:

- `scripts/alpha_release_smoke.sh`

What it validates:

1. Build `jade` and `jadec` binaries.
2. Run tests/build/run/package for `examples/alpha_release_demo`.
3. Run local package publish/include/fetch flow using a temporary git package.
4. Attempt cross-target build (`wasm32-wasi --standalone`).
5. Run focused actor/store benchmark subset.

## Remaining Alpha Risks (Non-blocking for this change set)

1. Cross-target builds still depend on external linker/toolchain availability.
2. Full performance baselining across all actor/store benchmarks should continue in CI/nightly.
3. Package publish currently uses git-tag workflow; remote registry UX is not yet implemented.
