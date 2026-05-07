# Rename Tooling — `jade` → `jinn`

Operator runbook for the rename described in [JINN.md](../../JINN.md).
Stdlib-only Python (≥3.10). All scripts are safe by default — they require
`--apply` to mutate the tree.

## Files

- `config.py` — single source of truth: substitution rules, file/dir
  inclusions, exclusions, allowlists. **Edit this file to tune scope.**
- `inventory.py` — read-only audit; reports every file and path that
  would be touched plus any "suspicious" compound tokens (e.g. `Jadeite`)
  that no rule covers and need human review.
- `rename.py` — applies the rename. Two phases: text rewrites and path
  renames. Run them as separate commits.
- `verify.py` — post-rename gate; CI exit code is 0 iff the tree is
  clean of forbidden patterns outside the allowlist.

## Standard procedure

```bash
# 0. Safety: make sure the working tree is clean.
git status

# 1. Take inventory; archive the report as the audit baseline.
python3 scripts/rename/inventory.py --out rename_inventory.json
# Review any "suspicious" tokens before proceeding.

# 2. Branch.
git checkout -b rename/jinn

# 3. Phase 1 — text rewrite (single commit).
python3 scripts/rename/rename.py --apply --text-only
git add -A
git commit -m "chore(rename): mechanical text substitution jade→jinn (no file moves)"

# 4. Phase 2 — file/dir renames using git mv.
python3 scripts/rename/rename.py --apply --paths-only --use-git-mv
git add -A
git commit -m "chore(rename): rename files and extensions (.jade→.jn)"

# 5. Manual phase — fix Cargo.toml bin paths, regen tree-sitter, etc.
#    See JINN.md §4 Phase 3.

# 6. Verify.
python3 scripts/rename/verify.py
echo "exit=$?"
```

## Recovery

Every phase is in its own commit; revert with `git revert`. If a dry-run
or partial apply leaves the tree in a weird state, `git restore .`
returns to a clean checkout.

## Tuning

To add a new rule (e.g. another extension), edit `config.py`:

- New text rule → append to `TEXT_RULES` (longer/more specific first).
- New path rule → append to `PATH_RULES`.
- Allowlist a file from rewriting → add its repo-relative path to
  `CONTENT_ALLOWLIST`.
- Skip an entire directory → add its basename to `EXCLUDED_DIRS`.

Re-run `inventory.py` after every edit to confirm the new behavior.

## CI integration (post-merge)

After Phase 5 (verification sweep) merges, add to CI:

```yaml
- name: Verify no jade leftovers
  run: python3 scripts/rename/verify.py
```

This prevents regressions.
