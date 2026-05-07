#!/usr/bin/env python3
"""Post-rename audit. Exits 0 iff no forbidden 'jade' references remain
outside the configured allowlist.

Usage:
    python3 scripts/rename/verify.py [--quiet]
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import config as cfg


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--quiet", action="store_true")
    args = parser.parse_args()

    leftover_files: list[tuple[str, list[tuple[int, str]]]] = []
    leftover_paths: list[str] = []

    for path in cfg.walk_repo():
        rel = cfg.path_rel(path)

        # Path-name leftovers (any path component still containing "jade",
        # case-insensitive, or a .jade/.jadei extension).
        for part in path.relative_to(cfg.REPO_ROOT).parts:
            if "jade" in part.lower() or part.endswith((".jade", ".jadei")):
                leftover_paths.append(rel)
                break

        if rel in cfg.CONTENT_ALLOWLIST:
            continue
        if path.suffix.lower() in cfg.BINARY_SUFFIXES:
            continue

        try:
            data = path.read_bytes()
        except OSError:
            continue
        if cfg.looks_binary(data):
            continue
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError:
            try:
                text = data.decode("latin-1")
            except Exception:
                continue

        hits: list[tuple[int, str]] = []
        for lineno, line in enumerate(text.splitlines(), 1):
            if cfg.LEFTOVER_PATTERN.search(line):
                hits.append((lineno, line.rstrip()))
        if hits:
            leftover_files.append((rel, hits))

    ok = not leftover_files and not leftover_paths

    if not args.quiet:
        if leftover_paths:
            print("FAIL: paths still containing 'jade' or .jade/.jadei extension:")
            for p in sorted(set(leftover_paths)):
                print(f"  {p}")
            print()
        if leftover_files:
            print("FAIL: files with leftover 'jade' references:")
            for rel, hits in leftover_files:
                print(f"  {rel}")
                for ln, line in hits[:5]:
                    print(f"    L{ln}: {line[:120]}")
                if len(hits) > 5:
                    print(f"    ... and {len(hits) - 5} more")
            print()
        if ok:
            print("OK: no forbidden 'jade' references remain.")

    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
