#!/usr/bin/env python3
"""
jade_syntax_migrate.py — Batch syntax migration for Jade source files.

Applies the syntax changes from syntax-audit.md across all .jade files.
Run with --dry-run first to preview changes.

Usage:
    python3 jade_syntax_migrate.py --dry-run          # preview only
    python3 jade_syntax_migrate.py --apply             # apply changes
    python3 jade_syntax_migrate.py --apply --phase 1   # apply only phase 1
    python3 jade_syntax_migrate.py --report            # count occurrences only
"""

import argparse
import os
import re
import sys
from pathlib import Path
from dataclasses import dataclass, field

JADE_ROOT = Path(__file__).parent
JADE_DIRS = ["std", "tests/programs", "tests", "benchmarks"]

# ─── Replacement Rules ────────────────────────────────────────────────

@dataclass
class Rule:
    name: str
    phase: int
    pattern: re.Pattern
    replacement: str  # str or callable
    description: str
    count: int = 0
    use_fn: bool = False

    def apply(self, text: str) -> tuple[str, int]:
        new_text, n = self.pattern.subn(self.replacement, text)
        self.count += n
        return new_text, n


def build_rules() -> list[Rule]:
    rules = []

    # ── Phase 1: return types  ->  →  returns ──────────────────────

    # Extern return types:  ) -> Type  →  ) returns Type
    rules.append(Rule(
        name="arrow_return_extern",
        phase=1,
        pattern=re.compile(r'\)\s*->\s*'),
        replacement=') returns ',
        description="Return type arrow '->' → 'returns' (after closing paren)",
    ))

    # ── Phase 2: type annotations  :  →  as  ─────────────────────

    # Parameter type annotations:  name: Type  →  name as Type
    # This matches:  word: TypeName  where TypeName starts with uppercase, %, &, or (
    # Pattern: identifier followed by : then a type
    rules.append(Rule(
        name="param_type_colon",
        phase=2,
        pattern=re.compile(r'(\b[a-z_][a-z0-9_]*)\s*:\s*(%?&?[A-Za-z(])'),
        replacement=r'\1 as \2',
        description="Type annotation ':' → 'as' (param: Type → param as Type)",
    ))

    # ── Phase 3: modulo  %  →  mod  ──────────────────────────────

    # Binary % (with spaces around it):  expr % expr  →  expr mod expr
    # Must NOT match %identifier (pointer prefix) or %i8 etc.
    rules.append(Rule(
        name="modulo_operator",
        phase=3,
        pattern=re.compile(r'(\S)\s+%\s+(\S)'),
        replacement=r'\1 mod \2',
        description="Modulo operator '%' → 'mod' (binary with spaces)",
    ))

    # ── Phase 4: isnt  →  neq  ───────────────────────────────────

    # Replace `isnt` with `neq` — word boundary match
    rules.append(Rule(
        name="isnt_to_neq",
        phase=4,
        pattern=re.compile(r'\bisnt\b'),
        replacement='neq',
        description="Not-equals 'isnt' → 'neq'",
    ))

    # ── Phase 5: empty parens in fn defs  *name()  → *name  ─────

    # Only at start of line (function definitions)
    # *name()  →  *name  (but NOT *fn() which is lambda)
    rules.append(Rule(
        name="empty_parens_fndef",
        phase=5,
        pattern=re.compile(r'^\*([a-zA-Z_][a-zA-Z0-9_]*)\(\)\s*$', re.MULTILINE),
        replacement=r'*\1',
        description="Remove empty parens from no-arg fn defs: *name() → *name",
    ))

    # Also inside type bodies (indented):  *name()  →  *name
    rules.append(Rule(
        name="empty_parens_method",
        phase=5,
        pattern=re.compile(r'^(\s+)\*([a-zA-Z_][a-zA-Z0-9_]*)\(\)\s*$', re.MULTILINE),
        replacement=r'\1*\2',
        description="Remove empty parens from no-arg method defs: *name() → *name",
    ))

    # ── Phase 6: remove %= lines entirely ────────────────────────

    # Remove lines containing %= (augmented modulo assignment is dropped)
    rules.append(Rule(
        name="remove_percent_eq",
        phase=6,
        pattern=re.compile(r'^.*%=.*\n', re.MULTILINE),
        replacement='',
        description="Remove %= augmented assignment lines (dropped from language)",
    ))

    return rules


# ─── File Discovery ───────────────────────────────────────────────────

def find_jade_files() -> list[Path]:
    files = []
    for d in JADE_DIRS:
        dirpath = JADE_ROOT / d
        if not dirpath.exists():
            continue
        for f in sorted(dirpath.rglob("*.jade")):
            files.append(f)
    # Also pick up top-level .jade files
    for f in sorted(JADE_ROOT.glob("*.jade")):
        files.append(f)
    return list(dict.fromkeys(files))  # dedupe, preserve order


# ─── Processing ───────────────────────────────────────────────────────

def process_file(filepath: Path, rules: list[Rule], apply: bool) -> list[str]:
    """Process one file. Returns list of change descriptions."""
    with open(filepath, 'r') as f:
        original = f.read()

    text = original
    changes = []

    for rule in rules:
        new_text, n = rule.apply(text)
        if n > 0:
            changes.append(f"  [{rule.name}] {n} replacement(s): {rule.description}")
            text = new_text

    if changes and apply:
        with open(filepath, 'w') as f:
            f.write(text)

    return changes


# ─── Reporting ────────────────────────────────────────────────────────

def report_patterns():
    """Count current occurrences of patterns that need changing."""
    files = find_jade_files()
    counts = {
        "colon_type_annotations": 0,
        "arrow_return_types": 0,
        "modulo_percent": 0,
        "isnt_keyword": 0,
        "empty_paren_fndefs": 0,
        "percent_eq_augmented": 0,
    }

    for f in files:
        text = f.read_text()
        lines = text.split('\n')
        for line in lines:
            stripped = line.split('#')[0]  # ignore comments
            # Colon type annotations
            counts["colon_type_annotations"] += len(re.findall(
                r'\b[a-z_][a-z0-9_]*\s*:\s*[%&A-Z(]', stripped))
            # Arrow return types
            counts["arrow_return_types"] += len(re.findall(r'\)\s*->', stripped))
            # Modulo (binary, not pointer prefix)
            counts["modulo_percent"] += len(re.findall(r'\S\s+%\s+\S', stripped))
            # isnt keyword
            counts["isnt_keyword"] += len(re.findall(r'\bisnt\b', stripped))
            # Empty paren fn defs
            if re.match(r'\s*\*[a-zA-Z_][a-zA-Z0-9_]*\(\)\s*$', line):
                counts["empty_paren_fndefs"] += 1
            # %= augmented assignment
            if '%=' in stripped:
                counts["percent_eq_augmented"] += 1

    print("\n=== Pattern Occurrence Report ===\n")
    for pattern, count in counts.items():
        status = "✓ CLEAN" if count == 0 else f"⚠ {count} occurrences"
        print(f"  {pattern:30s}  {status}")
    print(f"\n  Total files scanned: {len(files)}")


# ─── Main ─────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(description="Jade syntax migration tool")
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--dry-run", action="store_true",
                       help="Preview changes without applying")
    group.add_argument("--apply", action="store_true",
                       help="Apply changes to files")
    group.add_argument("--report", action="store_true",
                       help="Report pattern occurrences only")
    parser.add_argument("--phase", type=int, choices=[1, 2, 3, 4, 5, 6],
                        help="Only apply rules from this phase")
    parser.add_argument("--file", type=str,
                        help="Only process this specific file")
    args = parser.parse_args()

    if args.report:
        report_patterns()
        return

    rules = build_rules()
    if args.phase:
        rules = [r for r in rules if r.phase == args.phase]

    if args.file:
        files = [Path(args.file)]
    else:
        files = find_jade_files()

    print(f"\n{'DRY RUN' if args.dry_run else 'APPLYING'} — "
          f"{len(rules)} rules, {len(files)} files\n")
    print(f"Phases: {sorted(set(r.phase for r in rules))}")
    print(f"Rules:")
    for r in rules:
        print(f"  Phase {r.phase}: {r.name} — {r.description}")
    print()

    total_changes = 0
    files_changed = 0

    for filepath in files:
        if not filepath.exists():
            continue

        changes = process_file(filepath, rules, apply=args.apply)

        if changes:
            relpath = filepath.relative_to(JADE_ROOT)
            print(f"{'WOULD CHANGE' if args.dry_run else 'CHANGED'}: {relpath}")
            for c in changes:
                print(c)
            total_changes += len(changes)
            files_changed += 1

    print(f"\n{'=' * 60}")
    print(f"Total: {total_changes} change operations across {files_changed} files")
    print(f"\nPer-rule totals:")
    for r in rules:
        if r.count > 0:
            print(f"  {r.name:30s}  {r.count} replacements")

    if args.dry_run:
        print(f"\nThis was a dry run. Use --apply to make changes.")
    else:
        print(f"\nChanges applied. Run 'cargo test' to verify.")
        print(f"Tip: git diff to review changes before committing.")


if __name__ == "__main__":
    main()
