#!/usr/bin/env python3
r"""Strip all comments (line, block, doc) from Rust source files.

Handles:
- "..." string literals with escapes
- b"..." byte strings
- r"...", r#"..."#, br#"..."# raw strings (arbitrary hash count)
- '...' char literals with escapes (\xNN, \u{...}, etc.)
- 'lifetime identifiers
- // line comments (preserves newline)
- /* nested block comments */
- /// and //! doc comments (treated as comments and removed)
"""
import sys
from pathlib import Path


def strip_comments(src: str) -> str:
    out = []
    i = 0
    n = len(src)
    while i < n:
        c = src[i]

        # Raw string detection: r"...", r#"..."#, br"..."
        # Must be checked before generic identifier handling.
        is_raw = False
        raw_start = i
        if c == 'b' and i + 1 < n and (src[i + 1] == 'r'):
            j = i + 2
            hashes = 0
            while j < n and src[j] == '#':
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                is_raw = True
                raw_body_start = j + 1
                raw_hashes = hashes
                raw_prefix_end = j + 1
        elif c == 'r':
            j = i + 1
            hashes = 0
            while j < n and src[j] == '#':
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                is_raw = True
                raw_body_start = j + 1
                raw_hashes = hashes
                raw_prefix_end = j + 1

        if is_raw:
            out.append(src[raw_start:raw_prefix_end])
            end_marker = '"' + ('#' * raw_hashes)
            idx = src.find(end_marker, raw_body_start)
            if idx == -1:
                out.append(src[raw_body_start:])
                return ''.join(out)
            out.append(src[raw_body_start:idx + len(end_marker)])
            i = idx + len(end_marker)
            continue

        # Byte string b"..."
        if c == 'b' and i + 1 < n and src[i + 1] == '"':
            out.append('b"')
            i += 2
            while i < n:
                ch = src[i]
                if ch == '\\':
                    out.append(ch)
                    if i + 1 < n:
                        out.append(src[i + 1])
                        i += 2
                    else:
                        i += 1
                elif ch == '"':
                    out.append('"')
                    i += 1
                    break
                else:
                    out.append(ch)
                    i += 1
            continue

        # Byte char b'x'
        if c == 'b' and i + 1 < n and src[i + 1] == "'":
            # delegate to char-literal logic below by treating prefix
            out.append('b')
            i += 1
            c = src[i]
            # fall through into char-literal block below

        # String literal "..."
        if c == '"':
            out.append('"')
            i += 1
            while i < n:
                ch = src[i]
                if ch == '\\':
                    out.append(ch)
                    if i + 1 < n:
                        out.append(src[i + 1])
                        i += 2
                    else:
                        i += 1
                elif ch == '"':
                    out.append('"')
                    i += 1
                    break
                else:
                    out.append(ch)
                    i += 1
            continue

        # Char literal vs lifetime
        if c == "'":
            # Char literal forms:
            #   'x'   (single char, not \ or ')
            #   '\..' (escape sequence)
            # Lifetime: 'ident (no closing quote)
            if i + 1 < n and src[i + 1] == '\\':
                # Escape — scan for closing '
                j = i + 2
                if j < n and src[j] == 'u' and j + 1 < n and src[j + 1] == '{':
                    k = src.find('}', j + 2)
                    if k == -1:
                        out.append(src[i])
                        i += 1
                        continue
                    j = k + 1
                elif j < n and src[j] == 'x':
                    j += 3  # \xNN
                else:
                    j += 1  # \n, \t, \\, \', \", \0, \r
                if j < n and src[j] == "'":
                    out.append(src[i:j + 1])
                    i = j + 1
                    continue
                # Malformed; emit as-is.
                out.append(src[i])
                i += 1
                continue
            elif (i + 2 < n and src[i + 2] == "'" and src[i + 1] != '\\'
                  and src[i + 1] != '\n'):
                # 'x'
                out.append(src[i:i + 3])
                i += 3
                continue
            else:
                # Lifetime — just emit the quote, identifier follows naturally.
                out.append(c)
                i += 1
                continue

        # Line comment
        if c == '/' and i + 1 < n and src[i + 1] == '/':
            j = src.find('\n', i)
            if j == -1:
                return ''.join(out)
            i = j  # keep the newline
            continue

        # Block comment (nested)
        if c == '/' and i + 1 < n and src[i + 1] == '*':
            depth = 1
            j = i + 2
            while j < n and depth > 0:
                if src[j] == '/' and j + 1 < n and src[j + 1] == '*':
                    depth += 1
                    j += 2
                elif src[j] == '*' and j + 1 < n and src[j + 1] == '/':
                    depth -= 1
                    j += 2
                else:
                    j += 1
            i = j
            continue

        out.append(c)
        i += 1

    return ''.join(out)


def main():
    paths = [Path(p) for p in sys.argv[1:]]
    changed = 0
    for p in paths:
        src = p.read_text(encoding='utf-8')
        new = strip_comments(src)
        if new != src:
            p.write_text(new, encoding='utf-8')
            changed += 1
    print(f"stripped: {changed}/{len(paths)} files")


if __name__ == '__main__':
    main()
