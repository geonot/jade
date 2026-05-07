"""Single source of truth for the jade→jinn rename.

All three scripts (inventory, rename, verify) import from here. Edit this
file (and only this file) to tune scope, exclusions, or substitution
rules. Pure stdlib; no third-party deps.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass
from pathlib import Path

# ---------------------------------------------------------------------------
# Repository root resolution
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parents[2]


# ---------------------------------------------------------------------------
# Substitution rules. Order matters: longer / more-specific first.
# Each rule is (compiled_regex, replacement, human_label).
#
# We use raw \b-anchored regexes for the bare word forms so that compound
# identifiers like "Jadeite" would NOT be silently rewritten — the inventory
# script reports them so a human can decide.
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Rule:
    pattern: re.Pattern[str]
    replacement: str
    label: str


TEXT_RULES: tuple[Rule, ...] = (
    # Extensions first (more specific than the bare word).
    Rule(re.compile(r"\.jadei\b"), ".jni", ".jadei→.jni"),
    Rule(re.compile(r"\.jade\b"), ".jn", ".jade→.jn"),
    # SCREAMING_SNAKE. Note `_` is a regex word char, so \bJADE\b will NOT
    # match "JADE" inside "JADE_RT_DIR". We therefore need explicit rules
    # for each contextual position.
    Rule(re.compile(r"\bJADE_"), "JINN_", "JADE_*"),
    Rule(re.compile(r"_JADE_"), "_JINN_", "*_JADE_*"),
    Rule(re.compile(r"_JADE\b"), "_JINN", "*_JADE"),
    Rule(re.compile(r"\bJADE\b"), "JINN", "JADE→JINN (bare)"),
    # PascalCase — bare "Jade" plus the well-known PascalCase prefix
    # (e.g., JadeParser, JadeVec). "Jadeite" etc. would NOT match here and
    # are surfaced by the inventory's "suspicious" classifier.
    Rule(re.compile(r"\bJade(?=[A-Z])"), "Jinn", "Jade* (PascalCase)"),
    Rule(re.compile(r"\bJade\b"), "Jinn", "Jade→Jinn (word)"),
    # snake_case identifiers and well-known compound names.
    Rule(re.compile(r"\bjadec\b"), "jinnc", "jadec→jinnc (binary, bare)"),
    Rule(re.compile(r"_jadec\b"), "_jinnc", "*_jadec (e.g. CARGO_BIN_EXE_jadec)"),
    Rule(re.compile(r"\bjadei\b"), "jni", "jadei→jni (interface, bare)"),
    Rule(re.compile(r"\bjadei_"), "jni_", "jadei_*"),
    Rule(re.compile(r"\blibjade"), "libjinn", "libjade*→libjinn*"),
    # Linker flags (`-ljade_rt`) appear as a single token; the `l` prefix
    # defeats `\bjade_`, so handle it explicitly.
    Rule(re.compile(r"-ljade_"), "-ljinn_", "-ljade_*"),
    Rule(re.compile(r"\bjade_"), "jinn_", "jade_*"),
    Rule(re.compile(r"_jade_"), "_jinn_", "*_jade_*"),
    Rule(re.compile(r"_jade\b"), "_jinn", "*_jade"),
    Rule(re.compile(r"\bjade\b"), "jinn", "jade→jinn (word)"),
)


# Path-component substitutions (applied to each part of a path).
PATH_RULES: tuple[Rule, ...] = (
    Rule(re.compile(r"\.jadei$"), ".jni", "*.jadei→*.jni"),
    Rule(re.compile(r"\.jade$"), ".jn", "*.jade→*.jn"),
    Rule(re.compile(r"\bJADE\b"), "JINN", "JADE→JINN (path)"),
    Rule(re.compile(r"\bJade\b"), "Jinn", "Jade→Jinn (path)"),
    Rule(re.compile(r"\bjade\b"), "jinn", "jade→jinn (path)"),
    Rule(re.compile(r"jade"), "jinn", "jade→jinn (path-substr)"),
)


# ---------------------------------------------------------------------------
# Inclusion / exclusion of files
# ---------------------------------------------------------------------------

# File extensions whose CONTENTS we will rewrite.
TEXT_EXTENSIONS: frozenset[str] = frozenset(
    {
        ".rs",
        ".toml",
        ".md",
        ".c",
        ".h",
        ".S",
        ".py",
        ".sh",
        ".bash",
        ".zsh",
        ".json",
        ".yaml",
        ".yml",
        ".jade",  # source files (also renamed by extension)
        ".ebnf",
        ".js",
        ".ts",
        ".tsx",
        ".mjs",
        ".cjs",
        ".html",
        ".css",
        ".txt",
        ".cfg",
        ".ini",
        ".tmLanguage",
        ".scm",  # tree-sitter query files
    }
)

# Files with these basenames are also rewritten, regardless of extension.
TEXT_BASENAMES: frozenset[str] = frozenset(
    {
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "rustfmt.toml",
        "clippy.toml",
        "Makefile",
        "Dockerfile",
        ".gitignore",
        ".gitattributes",
        ".editorconfig",
        "README",
        "LICENSE",
        "NOTICE",
        "jade.pkg",  # will be renamed to jinn.pkg
        "jinn.pkg",  # post-rename
    }
)

# Directories never traversed.
EXCLUDED_DIRS: frozenset[str] = frozenset(
    {
        ".git",
        "target",
        "node_modules",
        ".venv",
        "venv",
        "__pycache__",
        ".mypy_cache",
        ".pytest_cache",
        ".idea",
        ".vscode",
        "dist",
        "build",
        "out",
    }
)

# Specific paths (relative to REPO_ROOT) whose CONTENTS are never rewritten,
# but the files themselves remain in place. The verifier also allowlists
# these (any leftover "jade" hit inside them is OK).
CONTENT_ALLOWLIST: frozenset[str] = frozenset(
    {
        "JINN.md",
        "CHANGELOG.md",
        "benchmarks/history.json",
        "benchmarks/results.json",
        "benchmarks/results_full.json",
        "benchmarks/results_pre_sim.json",
        "benchmarks/results.csv",
        "scripts/rename/config.py",
        "scripts/rename/inventory.py",
        "scripts/rename/rename.py",
        "scripts/rename/verify.py",
        "scripts/rename/README.md",
    }
)

# Filename suffixes that are always treated as binary (skip).
BINARY_SUFFIXES: frozenset[str] = frozenset(
    {
        ".png",
        ".jpg",
        ".jpeg",
        ".gif",
        ".ico",
        ".webp",
        ".pdf",
        ".vsix",
        ".zip",
        ".gz",
        ".tar",
        ".tgz",
        ".xz",
        ".bz2",
        ".so",
        ".dylib",
        ".dll",
        ".a",
        ".o",
        ".rlib",
        ".wasm",
        ".class",
        ".jar",
        ".woff",
        ".woff2",
        ".ttf",
        ".otf",
    }
)


# ---------------------------------------------------------------------------
# Detection regexes used by the verifier.
# ---------------------------------------------------------------------------

# Anything that still looks like the old name. The verifier raises on any
# match outside CONTENT_ALLOWLIST.
LEFTOVER_PATTERN = re.compile(r"\.jadei\b|\.jade\b|\bJADE\b|\bJade\b|\bjade\b|jade_")


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def is_excluded_dir(name: str) -> bool:
    return name in EXCLUDED_DIRS


def is_text_target(path: Path) -> bool:
    """True iff `path`'s contents should be rewritten."""
    rel = path_rel(path)
    if rel in CONTENT_ALLOWLIST:
        return False
    if path.suffix.lower() in BINARY_SUFFIXES:
        return False
    if path.name in TEXT_BASENAMES:
        return True
    return path.suffix in TEXT_EXTENSIONS


def path_rel(path: Path) -> str:
    """Return path relative to REPO_ROOT using forward slashes."""
    try:
        return path.resolve().relative_to(REPO_ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def looks_binary(data: bytes) -> bool:
    """Heuristic: NUL byte in first 8 KiB → binary."""
    return b"\x00" in data[:8192]


def walk_repo(root: Path = REPO_ROOT):
    """Yield every file under `root` not inside an excluded directory."""
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if not is_excluded_dir(d)]
        for fn in filenames:
            yield Path(dirpath) / fn


def apply_text_rules(text: str) -> tuple[str, dict[str, int]]:
    """Apply TEXT_RULES to `text`. Returns (new_text, per-rule counts)."""
    counts: dict[str, int] = {}
    out = text
    for rule in TEXT_RULES:
        out, n = rule.pattern.subn(rule.replacement, out)
        if n:
            counts[rule.label] = counts.get(rule.label, 0) + n
    return out, counts


def apply_path_rules(name: str) -> tuple[str, dict[str, int]]:
    """Apply PATH_RULES to a single path component."""
    counts: dict[str, int] = {}
    out = name
    for rule in PATH_RULES:
        out, n = rule.pattern.subn(rule.replacement, out)
        if n:
            counts[rule.label] = counts.get(rule.label, 0) + n
    return out, counts


def rename_path_components(path: Path) -> Path:
    """Apply PATH_RULES to every component of `path` below REPO_ROOT."""
    rel = path.resolve().relative_to(REPO_ROOT)
    new_parts = []
    for part in rel.parts:
        new_part, _ = apply_path_rules(part)
        new_parts.append(new_part)
    return REPO_ROOT.joinpath(*new_parts)
