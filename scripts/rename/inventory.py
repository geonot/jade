#!/usr/bin/env python3
"""Read-only inventory of every jade→jinn rewrite that would be applied.

Usage:
    python3 scripts/rename/inventory.py [--out FILE] [--quiet]

Emits to stdout (and optionally JSON to --out):
    - Files whose contents would be rewritten (with per-rule hit counts).
    - Files / directories that would be renamed (old → new).
    - "Suspicious" hits: identifiers like Jadeite or jadeish that match a
      compound pattern (no rule covers them; needs human review).
    - Binary or excluded files that contain the byte sequence "jade"
      (purely informational).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from pathlib import Path

import config as cfg


SUSPICIOUS = re.compile(r"[A-Za-z_][A-Za-z0-9_]*[Jj][Aa][Dd][Ee][A-Za-z0-9_]*|[Jj][Aa][Dd][Ee][A-Za-z0-9_]+")


def scan_file(path: Path) -> dict:
    """Return inventory record for one file."""
    rec: dict = {
        "path": cfg.path_rel(path),
        "rewritten": False,
        "would_rename": False,
        "rule_hits": {},
        "suspicious": [],
        "new_path": None,
    }

    # Path rename?
    new_path = cfg.rename_path_components(path)
    if new_path != path:
        rec["would_rename"] = True
        rec["new_path"] = cfg.path_rel(new_path)

    if not cfg.is_text_target(path):
        return rec

    try:
        data = path.read_bytes()
    except OSError as e:
        rec["error"] = f"read failed: {e}"
        return rec

    if cfg.looks_binary(data):
        return rec

    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        try:
            text = data.decode("latin-1")
        except Exception as e:  # pragma: no cover
            rec["error"] = f"decode failed: {e}"
            return rec

    new_text, counts = cfg.apply_text_rules(text)
    if counts:
        rec["rewritten"] = True
        rec["rule_hits"] = counts

    # Detect compound identifiers no rule covers by checking what `jade`
    # tokens REMAIN after the rewrite. Anything still present is a true
    # coverage gap that needs human review.
    seen_suspicious: set[str] = set()
    for m in re.finditer(r"[A-Za-z_][A-Za-z0-9_]*[Jj][Aa][Dd][Ee][A-Za-z0-9_]*|[Jj][Aa][Dd][Ee][A-Za-z0-9_]+", new_text):
        seen_suspicious.add(m.group(0))
    if seen_suspicious:
        rec["suspicious"] = sorted(seen_suspicious)

    return rec


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, help="Write JSON report to this path.")
    parser.add_argument("--quiet", action="store_true", help="Suppress stdout summary.")
    args = parser.parse_args()

    files: list[dict] = []
    for path in cfg.walk_repo():
        rec = scan_file(path)
        if rec["rewritten"] or rec["would_rename"] or rec["suspicious"] or rec.get("error"):
            files.append(rec)

    summary: dict = {
        "repo_root": str(cfg.REPO_ROOT),
        "files_scanned": True,
        "files_with_text_rewrites": sum(1 for r in files if r["rewritten"]),
        "files_to_rename": sum(1 for r in files if r["would_rename"]),
        "suspicious_files": sum(1 for r in files if r["suspicious"]),
        "rule_totals": defaultdict(int),
    }
    for rec in files:
        for label, n in rec["rule_hits"].items():
            summary["rule_totals"][label] += n
    summary["rule_totals"] = dict(summary["rule_totals"])

    report = {"summary": summary, "files": files}

    if args.out:
        args.out.write_text(json.dumps(report, indent=2, sort_keys=True))

    if not args.quiet:
        print(f"Repo root: {cfg.REPO_ROOT}")
        print(f"Files with text rewrites: {summary['files_with_text_rewrites']}")
        print(f"Files to rename:          {summary['files_to_rename']}")
        print(f"Files with suspicious compound identifiers: {summary['suspicious_files']}")
        print()
        print("Rule totals:")
        for label, n in sorted(summary["rule_totals"].items(), key=lambda kv: -kv[1]):
            print(f"  {label:30s} {n:>6d}")

        if summary["suspicious_files"]:
            print()
            print("Suspicious tokens (need human review before --apply):")
            for rec in files:
                if rec["suspicious"]:
                    print(f"  {rec['path']}: {', '.join(rec['suspicious'])}")

        if args.out:
            print()
            print(f"Full report written to {args.out}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
