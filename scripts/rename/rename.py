#!/usr/bin/env python3
"""Apply the jade→jinn rename to the working tree.

Defaults to dry-run. Pass --apply to actually modify files.

Phases (independently selectable):
    --text-only   Only rewrite file CONTENTS (no path/dir renames).
    --paths-only  Only rename files/directories.
    (default)     Both, in order: text first, then paths.

Path-rename mode:
    --use-git-mv  Use `git mv` instead of os.rename (preserves history).

Usage:
    # Preview everything:
    python3 scripts/rename/rename.py
    # Phase 1 (commit between phases):
    python3 scripts/rename/rename.py --apply --text-only
    # Phase 2:
    python3 scripts/rename/rename.py --apply --paths-only --use-git-mv
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

import config as cfg


def rewrite_text(apply: bool) -> tuple[int, int]:
    """Walk the tree and rewrite contents. Returns (files_changed, hits)."""
    files_changed = 0
    total_hits = 0
    for path in cfg.walk_repo():
        if not cfg.is_text_target(path):
            continue
        try:
            data = path.read_bytes()
        except OSError:
            continue
        if cfg.looks_binary(data):
            continue
        try:
            text = data.decode("utf-8")
            encoding = "utf-8"
        except UnicodeDecodeError:
            text = data.decode("latin-1")
            encoding = "latin-1"

        new_text, counts = cfg.apply_text_rules(text)
        if not counts:
            continue

        files_changed += 1
        total_hits += sum(counts.values())
        rel = cfg.path_rel(path)
        hits = ", ".join(f"{k}:{v}" for k, v in counts.items())
        print(f"  TEXT  {rel}  ({hits})")

        if apply:
            path.write_bytes(new_text.encode(encoding))

    return files_changed, total_hits


def collect_path_renames() -> list[tuple[Path, Path]]:
    """Return a list of (old_path, new_path) for files AND directories.

    Sorted deepest-first so directories rename after their children.
    """
    moves: list[tuple[Path, Path]] = []

    # Walk and collect every file + dir under repo root, skipping excluded dirs.
    seen: set[Path] = set()
    for dirpath, dirnames, filenames in os.walk(cfg.REPO_ROOT):
        dirnames[:] = [d for d in dirnames if not cfg.is_excluded_dir(d)]
        d = Path(dirpath)
        if d != cfg.REPO_ROOT:
            seen.add(d)
        for fn in filenames:
            seen.add(d / fn)

    for old in seen:
        new = cfg.rename_path_components(old)
        if new != old:
            moves.append((old, new))

    # Deepest first.
    moves.sort(key=lambda pair: len(pair[0].parts), reverse=True)
    return moves


def rename_paths(apply: bool, use_git_mv: bool) -> int:
    moves = collect_path_renames()
    for old, new in moves:
        rel_old = cfg.path_rel(old)
        rel_new = cfg.path_rel(new)
        print(f"  MOVE  {rel_old}  ->  {rel_new}")
        if not apply:
            continue
        new.parent.mkdir(parents=True, exist_ok=True)
        if use_git_mv:
            # `git mv` requires the source be tracked. If it isn't, fall back
            # to os.rename so untracked files still move.
            res = subprocess.run(
                ["git", "mv", "-f", str(old), str(new)],
                cwd=cfg.REPO_ROOT,
                capture_output=True,
                text=True,
            )
            if res.returncode != 0:
                print(
                    f"    git mv failed ({res.stderr.strip()}); falling back to os.rename",
                    file=sys.stderr,
                )
                os.replace(old, new)
        else:
            os.replace(old, new)
    return len(moves)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="Actually modify the tree.")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--text-only", action="store_true")
    mode.add_argument("--paths-only", action="store_true")
    parser.add_argument(
        "--use-git-mv",
        action="store_true",
        help="Use 'git mv' for path renames so blame/history is preserved.",
    )
    args = parser.parse_args()

    if not args.apply:
        print("=== DRY RUN (use --apply to write changes) ===")

    do_text = not args.paths_only
    do_paths = not args.text_only

    if do_text:
        print("\n--- TEXT REWRITES ---")
        files_changed, hits = rewrite_text(apply=args.apply)
        print(f"\nText: {files_changed} files, {hits} substitutions")

    if do_paths:
        print("\n--- PATH RENAMES ---")
        n = rename_paths(apply=args.apply, use_git_mv=args.use_git_mv)
        print(f"\nPaths: {n} renames")

    if not args.apply:
        print("\n(no changes written — re-run with --apply)")

    return 0


if __name__ == "__main__":
    sys.exit(main())
