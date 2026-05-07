# JINN — Rename Plan: `jade` → `jinn`

> Status: **Plan / Pre-execution**. No code changes yet. This document defines the
> complete scope, naming map, phased execution plan, tooling, risks, and
> verification checklist for renaming the language from **Jade** to **Jinn**
> (file extension `.jade` → `.jn`) prior to the alpha release.

---

## 1. Why

The name "Jade" is taken by an established programming/templating ecosystem
(notably the Jade/Pug HTML template language and several other tools on
crates.io and npm). To avoid trademark, search-engine, and packaging
collisions, the language is being renamed to **Jinn** before the alpha cuts
public artifacts (binaries, VS Code extension, docs site, crates).

This rename is a **one-shot, repo-wide, breaking change**. After it lands,
all source files use the new extension and identifiers; old `.jade` files
will not be accepted by the compiler.

---

## 2. Naming Map (authoritative)

All substitutions preserve case where the source token is alphabetic.

| Old                       | New                       | Notes                                                    |
| ------------------------- | ------------------------- | -------------------------------------------------------- |
| `jade`                    | `jinn`                    | snake_case / lowercase identifiers, paths, package names |
| `Jade`                    | `Jinn`                    | PascalCase types, prose                                  |
| `JADE`                    | `JINN`                    | SCREAMING_SNAKE env vars, macros                         |
| `.jade` (file extension)  | `.jn`                     | Source files                                             |
| `.jadei` (interface cache)| `.jni`                    | Compiled interface; see §7 risk note re: JNI namespace   |
| `jade.pkg` (manifest name)| `jinn.pkg`                | Package manifest filename                                |
| `jade_rt` (runtime lib)   | `jinn_rt`                 | C runtime archive `libjinn_rt.a`                         |
| `jade_ssl`                | `jinn_ssl`                | Optional TLS module                                      |
| `jade_sqlite`             | `jinn_sqlite`             | Optional sqlite module                                   |
| `jadec` (Cargo bin)       | `jinnc`                   | Compiler binary                                          |
| `jade` (Cargo bin)        | `jinn`                    | User-facing CLI wrapper                                  |
| `jadec-lsp` (Cargo bin)   | `jinnc-lsp`               | Language server                                          |
| `jade-lang` (VSCode id)   | `jinn-lang`               | Extension language id                                    |
| `source.jade-lang`        | `source.jinn-lang`        | TextMate scope name                                      |
| `tree-sitter-jade`        | `tree-sitter-jinn`        | Grammar package + dir                                    |
| `vscode-jade/`            | `vscode-jinn/`            | Extension dir                                            |
| `~/.cache/jade/`          | `~/.cache/jinn/`          | User cache directory                                     |
| `JADE_RT_DIR` etc.        | `JINN_RT_DIR` etc.        | All env vars (full list in §3.3)                         |
| `__jade_*` C symbols      | `__jinn_*`                | All emitted runtime helpers                              |
| `jade_wal_*` FFI symbols  | `jinn_wal_*`              | All `extern "C"` functions in runtime                    |

### 2.1 Cased substitution semantics

The rewriter MUST treat `jade`/`Jade`/`JADE` as three separate tokens. It
MUST NOT do a naive case-insensitive replace, because a regex like
`/jade/i → jinn` would turn `JADE` into `jinn`. The Python tooling in
§5 implements this correctly.

### 2.2 Allowed exceptions (do **not** rewrite)

These occurrences must be preserved verbatim:

- `CHANGELOG.md` historical entries (the rename itself should add a new entry).
- `benchmarks/history.json`, `benchmarks/results*.json`, `benchmarks/results.csv` —
  historical perf data keyed by old benchmark filenames. Add a one-line note,
  do not mutate.
- `target/`, `.git/`, `node_modules/`, `*.vsix`, `.venv/`, generated `*.lock`
  beyond Cargo.lock — never rewrite.
- Vendored third-party sources (none currently, but reserved).
- Any `.bak`/`.orig` files left by previous rewrites.
- Git author names, commit messages of past commits (history is immutable).
- Strings inside embedded test fixtures whose intent is to test the literal
  string `jade` (none currently identified; the `verify` script will surface
  any).

---

## 3. Inventory (current scope, measured)

Numbers below come from a pre-rename survey of `HEAD` excluding `target/`
and `.git/`.

### 3.1 Files renamed by extension

| Extension | Count |
| --------- | ----- |
| `.jade`   | 192   |
| `.jadei`  | (generated; cache only — handled by clearing user cache) |

### 3.2 Files / directories renamed by path

```
jade.md                      → jinn.md
jade.ebnf                    → jinn.ebnf
hello.jade                   → hello.jn
the-way-of-jade.md           → the-way-of-jinn.md
src/bin/jade.rs              → src/bin/jinn.rs
runtime/jade_rt.h            → runtime/jinn_rt.h
vscode-jade/                 → vscode-jinn/
  syntaxes/jade.tmLanguage.json → syntaxes/jinn.tmLanguage.json
  jade-lang-0.1.0.vsix       → (delete; rebuild as jinn-lang-*.vsix)
tree-sitter-jade/            → tree-sitter-jinn/
examples/**/jade.pkg         → examples/**/jinn.pkg
benchmarks/*.jade            → benchmarks/*.jn (36 files)
std/*.jade                   → std/*.jn
examples/**/*.jade           → examples/**/*.jn
tests/**/*.jade              → tests/**/*.jn (if any)
docs/**/*.jade               → docs/**/*.jn (if any)
```

### 3.3 Environment variables (build / runtime)

Found in source today; all must be renamed atomically across
`build.rs`, Rust, C, shell, and docs:

```
JADE_RT_DIR                 → JINN_RT_DIR
JADE_HAS_SSL                → JINN_HAS_SSL
JADE_HAS_SQLITE             → JINN_HAS_SQLITE
JADE_DUMP_IR                → JINN_DUMP_IR
JADE_DEBUG_MIR_CODEGEN      → JINN_DEBUG_MIR_CODEGEN
JADE_ALLOW_NON_HTTPS_DEPS   → JINN_ALLOW_NON_HTTPS_DEPS
JADE_ALLOW_SHELL            → JINN_ALLOW_SHELL
JADE_STACK_SIZE             → JINN_STACK_SIZE
JADE_GUARD_SIZE             → JINN_GUARD_SIZE
JADE_DEQUE_INIT_CAP         → JINN_DEQUE_INIT_CAP
JADE_STORE_CHUNK            → JINN_STORE_CHUNK
JADE_WAL_SYNC               → JINN_WAL_SYNC
JADE_WAL_SYNC_NONE          → JINN_WAL_SYNC_NONE
JADE_WAL_SYNC_FSYNC         → JINN_WAL_SYNC_FSYNC
JADE_WAL_SYNC_FDATASYNC     → JINN_WAL_SYNC_FDATASYNC
JADE_WAL_SYNC_GROUP         → JINN_WAL_SYNC_GROUP
JADE_SUP_MAX_RESTARTS       → JINN_SUP_MAX_RESTARTS
JADE_SUP_ONE_FOR_ONE        → JINN_SUP_ONE_FOR_ONE
JADE_SUP_ONE_FOR_ALL        → JINN_SUP_ONE_FOR_ALL
JADE_SUP_REST_FOR_ONE       → JINN_SUP_REST_FOR_ONE
JADE_CORO_READY/RUNNING/SUSPENDED/DONE → JINN_CORO_*
```

The plain-text rule `JADE → JINN` covers all of these; this list exists
so reviewers can sanity-check that nothing important was missed.

### 3.4 Cargo binaries

`Cargo.toml` (root):

```toml
name = "jadec"          → "jinnc"
default-run = "jadec"   → "jinnc"

[[bin]] name = "jadec"      → "jinnc"     path = src/main.rs
[[bin]] name = "jade"       → "jinn"      path = src/bin/jade.rs → src/bin/jinn.rs
[[bin]] name = "jadec-lsp"  → "jinnc-lsp" path = src/lsp/main.rs
```

Downstream impact: every script that invokes `target/debug/jadec` or
`cargo run --bin jadec` must be updated. The tooling in §5 finds and
patches these.

### 3.5 Volume estimate

| Category                              | Count (approx.) |
| ------------------------------------- | --------------- |
| Files containing `jade`/`Jade`/`JADE` | ~195            |
| `jade` (lowercase) occurrences        | ~4,567          |
| `Jade` occurrences                    | ~410            |
| `JADE` occurrences                    | ~135            |
| `\.jade\b` extension references       | ~261            |
| Files renamed by extension            | 192             |
| Files renamed by path                 | ~12             |
| Directories renamed                   | 2               |

---

## 4. Phased Execution Plan

The rename is sequenced so each phase leaves the tree in a buildable state
**at the end of the phase** (some intermediate states inside a phase will
not build; that is expected — do not commit mid-phase).

### Phase 0 — Preparation (no code changes)

1. Land this `JINN.md`.
2. Land tooling under `scripts/rename/` (this PR).
3. Run `python3 scripts/rename/inventory.py --out rename_inventory.json`
   on a clean tree; archive the JSON as the audit baseline.
4. Announce a code freeze window (no merges into the rename target branch
   while rename is in flight).
5. Cut a long-lived branch `rename/jinn` from `main`.

### Phase 1 — Text rewrites (single mechanical commit)

On `rename/jinn`:

1. `python3 scripts/rename/rename.py --apply --text-only`
2. `cargo check` will fail because Cargo bin names + filenames no longer
   match. That is expected; do **not** try to fix by hand here. Go to
   Phase 2.
3. Commit: `chore(rename): mechanical text substitution jade→jinn (no file moves)`

### Phase 2 — Path & extension renames (mechanical commit)

1. `python3 scripts/rename/rename.py --apply --paths-only --use-git-mv`
   - Renames `*.jade` → `*.jn`
   - Renames `*.jadei` → `*.jni` (if any tracked)
   - Renames the named files/dirs in §3.2
   - Uses `git mv` so history is preserved.
2. Commit: `chore(rename): rename files and extensions (.jade→.jn, .jadei→.jni)`

### Phase 3 — Build/manifest fixes

These are intentionally manual review (small, high-impact):

1. `Cargo.toml` package + bin sections (already partly rewritten; verify).
2. `build.rs` library names (`jade_rt` → `jinn_rt`, `jade_ssl` → `jinn_ssl`,
   `jade_sqlite` → `jinn_sqlite`) — confirm the rewrite matched.
3. `vscode-jinn/package.json`: `name`, `displayName`, `publisher`,
   `language id`, `aliases`, `extensions: [".jn"]`, scope name.
4. `vscode-jinn/syntaxes/jinn.tmLanguage.json`: `scopeName` field.
5. `tree-sitter-jinn/grammar.js`: grammar `name`. Re-run
   `tree-sitter generate` and commit the regenerated parser.
6. `tree-sitter-jinn/package.json`: package name and `tree-sitter.json`
   entries; rename `queries/` filenames if any embed `jade`.
7. `runtime/jinn_rt.h` include guard macro (`JADE_RT_H` → `JINN_RT_H`).
8. Any shell scripts under `scripts/` that hard-code paths.
9. `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml` — usually no change.
10. `tests/`, `benchmarks/run_benchmarks.py`, CI workflows — update binary
    names and paths.
11. Run `cargo build --workspace --all-targets` to a clean compile.
12. Commit: `build(rename): finalize manifests, runtime headers, build.rs`

### Phase 4 — Test & runtime validation

1. `cargo test --workspace` must pass.
2. `cargo clippy --workspace --all-targets -- -D warnings` clean.
3. `bash scripts/preflight.sh` (after that script itself is updated).
4. `bash scripts/alpha_release_smoke.sh` end-to-end.
5. `python3 run_benchmarks.py --quick` — verify benchmarks still execute
   (results comparison vs. `history.json` is **expected to break** because
   benchmark filenames changed; document in commit message).
6. Manual smoke: `target/release/jinnc hello.jn -o /tmp/hello && /tmp/hello`.
7. Manual smoke: VS Code extension loads and highlights a `.jn` file.
8. Manual smoke: tree-sitter playground parses a `.jn` file.

### Phase 5 — Verification sweep

1. `python3 scripts/rename/verify.py` must exit 0.
   - The verifier walks the tree (excluding allowlisted paths) and fails
     on any remaining `[Jj]ade|JADE|\.jade\b|\.jadei\b` occurrence.
2. Manually scan the verifier's allowlist hits and confirm each is
   intentional.
3. Update `CHANGELOG.md` with a "BREAKING: renamed to Jinn" entry that
   intentionally references both old and new names.

### Phase 6 — External coordination (parallelizable with Phase 5)

1. Reserve `jinn-lang` / `jinnc` on:
   - crates.io (publish a `0.0.0-rename-placeholder` if needed)
   - npm (`@jinn-lang/tree-sitter-jinn`)
   - VS Code Marketplace (publisher + extension id)
   - GitHub: rename repo `jade` → `jinn`; set the redirect; update remote
     URL in local clones.
2. DNS / website: register and point `jinn-lang.org` (or chosen domain).
   Old domain (if any) should 301 to new.
3. Update social handles, `README.md` badges, CI badge URLs.

### Phase 7 — Merge and tag

1. Open PR `rename/jinn → main`. Required reviewers: at least one
   maintainer plus one person who runs the smoke scripts on a fresh clone.
2. Squash-merge with title:
   `chore!: rename language from Jade to Jinn (extension .jade→.jn)`
3. Tag `v0.0.0-jinn-rename` immediately on `main`.
4. Lift the freeze.

---

## 5. Tooling

All scripts live under `scripts/rename/` and require only Python ≥3.10
(stdlib only — no third-party dependencies, runs in CI).

| Script                              | Purpose                                                    |
| ----------------------------------- | ---------------------------------------------------------- |
| `scripts/rename/config.py`          | Single source of truth: substitution rules + exclusions.   |
| `scripts/rename/inventory.py`       | Walks the tree and emits a JSON report of every file/path/identifier that would be touched. Read-only. |
| `scripts/rename/rename.py`          | Executes the rename. Supports `--dry-run` (default), `--apply`, `--text-only`, `--paths-only`, `--use-git-mv`. |
| `scripts/rename/verify.py`          | Post-rename audit. Exits non-zero if any forbidden pattern remains outside the allowlist. |
| `scripts/rename/README.md`          | Operator runbook (commands, expected output, recovery).    |

### 5.1 Substitution algorithm (text)

For each file selected by the include list:

1. Skip if path matches any exclusion pattern.
2. Skip if file is binary (NUL byte detection in first 8 KiB).
3. Apply substitutions in this **fixed order** (longer/more-specific first
   to avoid double-rewrites):

   ```
   .jadei  → .jni
   .jade   → .jn
   JADE    → JINN
   Jade    → Jinn
   jade    → jinn
   ```

4. Cased patterns use word-boundary-aware matching where ambiguity exists
   (e.g., `Jadeite` should not become `Jinnite` — the verifier surfaces
   any "compound" hits before apply for human review). Today there are
   no such words in-tree, but the script is defensive.

### 5.2 Path renames

Same five rules applied to each path component (basename and directory
names). Performed bottom-up (deepest first) so directories rename
correctly. When `--use-git-mv` is set, every move goes through `git mv`
to preserve blame; otherwise `os.rename` is used (suitable for unstaged
trees).

### 5.3 Exclusions (built into config.py)

```
target/, .git/, node_modules/, .venv/, **/__pycache__/, **/*.vsix,
**/*.lock except Cargo.lock, **/*.png, **/*.jpg, **/*.gif, **/*.ico,
benchmarks/history.json, benchmarks/results*.json, benchmarks/results.csv,
CHANGELOG.md, JINN.md (this file is the canonical record and discusses both names)
```

### 5.4 CI integration

After Phase 5 lands, `scripts/rename/verify.py` is added as a CI gate so
no future commit can reintroduce `jade` outside the allowlist.

---

## 6. Verification Checklist (gate to merging Phase 7)

- [ ] `python3 scripts/rename/verify.py` exits 0.
- [ ] `find . -name '*.jade' -not -path './target/*' -not -path './.git/*'`
      returns **zero** results.
- [ ] `find . -iname '*jade*' -not -path './target/*' -not -path './.git/*'`
      returns **zero** results (or only allowlisted files).
- [ ] `cargo build --workspace --release` succeeds.
- [ ] `cargo test --workspace` succeeds.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `bash scripts/preflight.sh` succeeds.
- [ ] `bash scripts/alpha_release_smoke.sh` succeeds.
- [ ] `target/release/jinnc hello.jn` builds and runs.
- [ ] `target/release/jinnc-lsp --version` runs.
- [ ] VS Code extension installs and tokenises `hello.jn`.
- [ ] tree-sitter generates and parses `hello.jn` without errors.
- [ ] Benchmark suite executes (perf delta logged separately).
- [ ] No `JADE_*` env var is read by any tool (grep `std::env::var`).
- [ ] `~/.cache/jade/` is no longer created at runtime; `~/.cache/jinn/` is.
- [ ] `CHANGELOG.md` has a BREAKING entry naming both old and new names.

---

## 7. Risks and Mitigations

| Risk                                                                          | Likelihood | Mitigation                                                                                                          |
| ----------------------------------------------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------- |
| FFI symbol mismatch: Rust still calls `jade_wal_open` while C exports `jinn_wal_open`. | High if not atomic | Single-commit text rewrite (Phase 1) covers Rust + C together; build immediately after Phase 3 catches any miss. |
| `.jni` interface extension confused with Java Native Interface in docs/search. | Medium     | Document in §2; if confusion in user testing, fall back to `.jnif` or `.ji` (one-line config change in `config.py`). |
| Stale artifact cache `~/.cache/jade/` collides with new binary.               | Medium     | Release notes instruct users to `rm -rf ~/.cache/jade/`; new binary writes only `~/.cache/jinn/`.                    |
| Existing user code with `.jade` files breaks silently.                        | High       | Compiler emits a hard error with hint: "files must end in `.jn`; rename `.jade`→`.jn`". Implement in Phase 3.        |
| Benchmarks `history.json` keyed by old filenames loses continuity.            | Certain    | Accepted; a one-time perf re-baseline runs after Phase 4. Old data archived as `benchmarks/history.pre-jinn.json`.   |
| Tree-sitter parser regeneration produces large diff that hides bugs.         | Medium     | Regenerate as a **separate commit** within Phase 3, with a clean diff against the prior parser.                      |
| External links (crates.io, marketplace, GitHub) point to old names.          | Certain    | GitHub repo redirect; crates.io placeholder; marketplace publisher rename; archived old VS Code ext with deprecation.|
| Mid-phase commit lands on `main`.                                             | Medium     | Use a dedicated branch with branch protection; PR template forbids partial rename merges.                            |
| Naive case-insensitive replace damages `JADE_*` macros.                       | High       | Tooling enforces three separate cased rules (§5.1); verifier double-checks.                                          |
| Word-boundary collisions (e.g., a future identifier `Jadeite`).               | Low        | Inventory script flags compound matches before apply.                                                                |

---

## 8. Rollback

If Phase 4 reveals a blocking issue:

1. The rename lives entirely on `rename/jinn`. Roll back by abandoning the
   branch — `main` is untouched.
2. If already merged: revert the squashed commit. Because every change was
   mechanical and tooling is checked in, regenerating after a fix is cheap.
3. Tags created during the rename stay; do not delete published tags.

---

## 9. Out of Scope (explicit)

- Renaming the language **conceptually** (syntax, keywords, semantics are
  unchanged).
- Re-architecting the runtime, compiler, or stdlib.
- Stylistic or organizational refactors that happen to touch the same
  files. Discipline: rename PR contains *only* the rename.
- Migrating user codebases. A migration tool (`jinn migrate <dir>`) is a
  follow-up, tracked separately.

---

## 10. Open Questions (resolve before Phase 0 closes)

1. Confirm `.jni` for interface files vs. `.jnif` / `.ji`. **Default: `.jni`.**
2. Confirm the user CLI binary stays named `jinn` (vs. consolidating to
   just `jinnc`). **Default: keep both.**
3. Confirm cache directory: `~/.cache/jinn/` (XDG) on Linux; macOS path
   convention TBD by maintainer.
4. Domain / org name registration owner.
5. Who runs the marketplace publisher rename (requires Microsoft account).
6. **On-disk magic constants** (see §11): rename now, or freeze for
   backwards-compat? **Default: rename now (pre-alpha = no installed
   base).**

---

## 11. On-disk file-format magics (explicit decision)

The runtime writes 7-byte ASCII magic headers at the front of each store
file. The inventory tool surfaces these as items requiring **explicit
human review** because the rename is a wire-format break — old store
files become unreadable to the new binary.

| File              | Magic       | Suggested new |
| ----------------- | ----------- | ------------- |
| `runtime/wal.c`   | `JADEWAL`   | `JINNWAL`     |
| `runtime/kv.c`    | `JADEKV`    | `JINNKV0`     |
| `runtime/index.c` | `JADEIDX`   | `JINNIDX`     |
| `runtime/column.c`| `JADECOL`   | `JINNCOL`     |
| `runtime/vector.c`| `JADEVEC`   | `JINNVEC`     |
| `runtime/fts.c`   | `JADEFTS`   | `JINNFTS`     |
| `runtime/bloom.c` | `JADEBLM`   | `JINNBLM`     |
| `runtime/version.c`| `JADEVER`  | `JINNVER`     |
| `runtime/migrate.c`| `JADEMIG`  | `JINNMIG`     |
| `runtime/<various>`| `JADESTR` (×4) | `JINNSTR` |
| Header word       | `JADEC`     | `JINNC0`      |

**Decision (default for pre-alpha):** rename. There is no installed user
base; old store files are exclusively developer test fixtures and can be
regenerated. The rewrite tooling **does not** rewrite these
automatically — it is left as a deliberate manual edit during Phase 3 so
the operator can simultaneously bump any in-store version byte that
gates compatibility checks. The verifier's allowlist must be updated to
permit `JADESTR` etc. only for the duration of Phase 3 work; once the
manual edit lands, the leftover-token list goes to zero.

---

*Last updated: pre-execution. Update with concrete decisions on §10 before
starting Phase 0.*
