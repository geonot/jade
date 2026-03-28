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
    replacement: str
    description: str
    count: int = 0

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

    # Standalone fn return:  *name(...) -> Type  (handled by above since it ends with ))
    # Function type in param:  (i64) -> i64  →  (i64) returns i64  (also handled by above)
    # But we also need: *string_builder() -> StringBuilder  (also handled)

    # ── Phase 2: type annotations  :  →  as  ─────────────────────

    # Parameter type annotations:  name: Type  →  name as Type
    # This matches:  word: TypeName  where TypeName starts with uppercase, %, &, or (
    # Must NOT match inside strings or comments
    # Must NOT match after 'extern' keyword lines (those are handled separately)
    # Pattern: identifier followed by : then a type
    rules.append(Rule(
        name="param_type_colon",
        phase=2,
        pattern=re.compile(r'(\b[a-z_][a-z0-9_]*)\s*:\s*(%?&?[A-Za-z(])'),
        replacement=r'\1 as \2',
        description="Type annotation ':' → 'as' (param: Type → param as Type)",
    ))

    # Struct field type annotations with same pattern (covered by above)

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

    # %= augmented assignment → mod=
    rules.append(Rule(
        name="modulo_assign",
        phase=3,
        pattern=re.compile(r'\s%=\s'),
        replacement=' mod= ',
        description="Modulo assignment '%=' → 'mod='",
    ))

    # ── Phase 4: empty parens in fn defs  *name()  → *name  ─────

    # Only at start of line (function definitions)
    # *name()  →  *name  (but NOT *fn() which is lambda)
    rules.append(Rule(
        name="empty_parens_fndef",
        phase=4,
        pattern=re.compile(r'^\*([a-zA-Z_][a-zA-Z0-9_]*)\(\)\s*$', re.MULTILINE),
        replacement=r'*\1',
        description="Remove empty parens from no-arg fn defs: *name() → *name",
    ))

    # Also inside type bodies (indented):  *name()  →  *name
    rules.append(Rule(
        name="empty_parens_method",
        phase=4,
        pattern=re.compile(r'^(\s+)\*([a-zA-Z_][a-zA-Z0-9_]*)\(\)\s*$', re.MULTILINE),
        replacement=r'\1*\2',
        description="Remove empty parens from no-arg method defs: *name() → *name",
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


def process_line_aware(filepath: Path, apply: bool) -> list[str]:
    """Handle replacements that need line-level context (comments, strings)."""
    # This function handles edge cases the regex rules can't:
    # - Don't replace inside comments
    # - Don't replace inside string literals
    # For now, the regex rules are good enough for most cases.
    # Edge cases can be handled manually.
    return []


# ─── Reporting ────────────────────────────────────────────────────────

def report_patterns():
    """Count current occurrences of patterns that need changing."""
    files = find_jade_files()
    counts = {
        "colon_type_annotations": 0,
        "arrow_return_types": 0,
        "modulo_percent": 0,
        "empty_paren_fndefs": 0,
        "enum_variant_parens": 0,
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
            counts["arrow_return_types"] += len(re.findall(r'->', stripped))
            # Modulo (binary, not pointer prefix)
            counts["modulo_percent"] += len(re.findall(r'\S\s+%\s+\S', stripped))
            # Empty paren fn defs
            if re.match(r'\s*\*[a-zA-Z_][a-zA-Z0-9_]*\(\)\s*$', line):
                counts["empty_paren_fndefs"] += 1
            # Enum variant parens
            counts["enum_variant_parens"] += len(re.findall(
                r'^\s+[A-Z][a-zA-Z]*\(', line))

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
    parser.add_argument("--phase", type=int, choices=[1, 2, 3, 4],
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

        # Reset rule counts for per-file tracking
        # (we track per-rule totals separately)
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
