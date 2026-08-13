# Strings & Unicode

This document is the **canonical, stability-pinned specification** for Jinn's
`String` type. It defines the text encoding, the meaning of length, indexing,
slicing, and iteration, and the boundary between the *character* view and the
*byte* view. The semantics here are exercised by the conformance suite in
[`tests/string_unicode.rs`](../tests/string_unicode.rs); a regression in any of
them fails CI. `String` is a **stable** surface (see
[`stdlib.md`](stdlib.md#stability-tiers)).

## Encoding: UTF-8

A `String` is an immutable-by-value sequence of bytes that is **guaranteed to
be valid UTF-8**. String literals in source are UTF-8; concatenation, slicing,
case mapping, replacement, and the other built-in transforms preserve UTF-8
validity when their inputs are valid.

Storage is small-string-optimised: strings up to 23 bytes live inline; longer
strings are heap-allocated. This is an implementation detail and carries no
observable semantics beyond performance.

## Two views: scalars and bytes

Jinn distinguishes the **character view** (Unicode scalar values) from the
**byte view** (UTF-8 code units). The defaults favour natural-language intent;
the byte view is always available explicitly.

| Surface | View | Meaning |
| --- | --- | --- |
| `s.length` | scalar | number of Unicode scalar values |
| `s.byte_count` | byte | number of UTF-8 bytes in storage |
| `s.char_at(i)` | byte | the UTF-8 byte at byte offset `i` (returns its `i64` code) |
| `s[i]` | byte | identical to `s.char_at(i)` |
| `s.slice(a, b)` | byte | substring over the byte range `[a, b)` |
| `s.is_empty` | — | true iff `s.byte_count == 0` (equivalently `s.length == 0`) |

### `length` — Unicode scalar count

`s.length` (and its alias `s.len()`) returns the number of **Unicode scalar
values**, not bytes. This is the count a human means by "how long is this
word": each accented letter or emoji counts once.

```
"héllo".length       # 5   (é is one scalar, two UTF-8 bytes)
"😀".length          # 1
"".length            # 0
```

The count is computed by walking the UTF-8 bytes and counting lead bytes (bytes
whose top two bits are not `10`). For valid UTF-8 this equals the scalar count.

### `byte_count` — storage length

`s.byte_count` returns the number of bytes the string occupies. For ASCII it
equals `s.length`; for multi-byte text it is larger.

```
"héllo".byte_count   # 6   (h e l l o = 5 + é's extra byte)
"😀".byte_count      # 4
```

`byte_count` is the unit used by `char_at`, indexing, `slice`, `find`,
`contains`, `starts_with`, and `ends_with`. It is the right length to pair with
byte-level iteration.

## Indexing and `char_at` are byte-based

`s[i]` and `s.char_at(i)` address the string by **byte offset** and return the
`i64` value of that single UTF-8 byte. This is the low-level building block for
parsers, hashing, and protocol code, and it composes with `byte(code)` (the
inverse: an `i64` byte code → a one-byte string containing `code & 255`,
unencoded).

Two builtins construct strings from integer codes, and they sit on opposite
sides of the character/byte divide:

- `chr(code)` treats `code` as a **Unicode scalar value** and returns its
  UTF-8 encoding: `chr(65)` is `"A"` (one byte), `chr(233)` is `"é"` (two
  bytes), `chr(128512)` is `"😀"` (four bytes). Codes above `0x10FFFF` and
  surrogate codes encode as U+FFFD (the replacement character). `chr` always
  returns valid UTF-8, so it is the inverse of a scalar decode, not of
  `char_at`.
- `byte(code)` returns a one-byte string holding `code & 255`, unencoded.
  It is the byte-level inverse of `char_at`, and the building block for
  assembling binary data into a `String` used as a byte buffer. A `byte(b)`
  with `b >= 0x80` is **not** valid UTF-8 on its own; like mid-scalar slices,
  such strings are outside the UTF-8 contract until the caller has assembled a
  valid sequence (or keeps the value in byte-buffer territory: `byte_count`,
  `char_at`, `slice`, and concatenation all remain byte-exact).

```
"héllo".char_at(0)   # 104  (= 'h')
"héllo".char_at(1)   # 195  (first byte of é)
"héllo".char_at(2)   # 169  (continuation byte of é)
```

A byte that is part of a multi-byte scalar is **not** itself a valid standalone
character; reassembling scalars from bytes is the caller's responsibility. The
default `.length`/`char_at` pairing is therefore only scalar-correct for ASCII.
For byte-exact iteration, pair `char_at` with `byte_count`:

```
for i from 0 to s.byte_count
    handle(s.char_at(i))
```

## Slicing is byte-based

`s.slice(a, b)` returns the substring spanning byte offsets `[a, b)`. Callers
are responsible for choosing offsets that fall on scalar boundaries; slicing in
the middle of a multi-byte scalar yields a string that is not valid UTF-8 and
is therefore outside the contract.

## Iteration

There is no implicit character iterator yet. Iterate explicitly over the byte
view (`for i from 0 to s.byte_count`), or split into substrings with
`s.split(delim)` / `s.lines()`, which return `Vec<String>` cut on byte-exact
delimiters.

## Comparison, search, and transforms

- Equality (`==`) and `find` / `contains` / `starts_with` / `ends_with`
  compare **byte sequences**. Two strings are equal iff their bytes are equal.
  There is no Unicode normalisation: `"é"` (one scalar) and `"é"` (e + combining
  accent) are distinct strings.
- `to_upper` / `to_lower` map **ASCII** letters only; bytes `>= 0x80` pass
  through unchanged. Full Unicode case folding is out of scope at this tier.
- `trim` / `trim_left` / `trim_right` strip the ASCII whitespace set
  (`' '`, `\t`, `\n`, `\r`).
- `replace`, `repeat`, `split`, `lines` operate on byte sequences and preserve
  UTF-8 validity when their inputs do.

## Stability summary

The following are pinned and may not change before `1.0` without a changelog
entry:

1. `String` is valid UTF-8, except for byte buffers deliberately assembled
   with `byte(code)` or mid-scalar slices, which are outside the contract
   until made valid.
2. `s.length` / `s.len()` is the Unicode **scalar** count.
3. `s.byte_count` is the **byte** count and is the unit for `char_at`, `s[i]`,
   `slice`, `find`, `contains`, `starts_with`, `ends_with`.
4. `s[i]` / `s.char_at(i)` return a single UTF-8 **byte** as `i64`.
5. `chr(code)` UTF-8-encodes a Unicode scalar; `byte(code)` emits a raw byte.
6. Equality and search are byte-exact, with no normalisation.
7. `to_upper` / `to_lower` are ASCII-only.
