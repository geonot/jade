#!/usr/bin/env python3
r"""Strip all comments from C sources (.c/.h) or Jinn sources (.jn).

C mode handles "..." strings with escapes, '...' char literals, // line
comments and /* block comments */. Jinn mode handles '...' and "..."
strings and # line comments. A trailing comment leaves its newline in
place; a whole-line comment removes the line entirely so no blank gap is
left behind.

Preprocessor lines, string contents and character literals are never
touched.
"""

import sys
from pathlib import Path


def strip_c(src: str) -> str:
    out = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        if c in '"\'':
            quote = c
            out.append(c)
            i += 1
            while i < n:
                ch = src[i]
                if ch == '\\' and i + 1 < n:
                    out.append(src[i:i + 2])
                    i += 2
                    continue
                out.append(ch)
                i += 1
                if ch == quote:
                    break
            continue
        if c == '/' and i + 1 < n and src[i + 1] == '/':
            j = src.find('\n', i)
            i = n if j == -1 else j
            continue
        if c == '/' and i + 1 < n and src[i + 1] == '*':
            j = src.find('*/', i + 2)
            i = n if j == -1 else j + 2
            continue
        out.append(c)
        i += 1
    return ''.join(out)


def strip_jn(src: str) -> str:
    out = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]
        if c in '"\'':
            quote = c
            out.append(c)
            i += 1
            while i < n:
                ch = src[i]
                if ch == '\\' and i + 1 < n:
                    out.append(src[i:i + 2])
                    i += 2
                    continue
                out.append(ch)
                i += 1
                if ch == quote:
                    break
            continue
        if c == '#':
            j = src.find('\n', i)
            i = n if j == -1 else j
            continue
        out.append(c)
        i += 1
    return ''.join(out)


def drop_blanked_lines(original: str, stripped: str) -> str:
    """Remove lines that held only a comment, keep genuine blank lines."""
    keep = []
    for orig, new in zip(original.split('\n'), stripped.split('\n')):
        if orig.strip() and not new.strip():
            continue
        keep.append(new.rstrip())
    return '\n'.join(keep)


def main() -> None:
    changed = 0
    paths = [Path(p) for p in sys.argv[1:]]
    for p in paths:
        src = p.read_text(encoding='utf-8')
        strip = strip_jn if p.suffix == '.jn' else strip_c
        new = drop_blanked_lines(src, strip(src))
        if not new.endswith('\n'):
            new += '\n'
        if new != src:
            p.write_text(new, encoding='utf-8')
            changed += 1
    print(f"stripped: {changed}/{len(paths)} files")


if __name__ == '__main__':
    main()
