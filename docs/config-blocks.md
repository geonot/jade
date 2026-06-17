# config blocks — declarative sugar that is real Jinn

**Status: decided spec.** This answers prereqs §4 and Q3. lamp.md §2.1 says the
manifest is "a restricted Jinn dialect." Decision (path **(a)** from prereqs
§4.2): a **general declarative config-block construct** parsed by the *real* Jinn
parser, desugaring to ordinary Jinn values (object constructors / maps / lists /
records). `project.jn` is its first consumer; the manifest restrictions are a
*validation lint over the parsed block*, not a second grammar. No second config
language, by construction.

---

## 0. The principle

A config block is **sugar for constructing a value.** Everything in `project.jn`
is a normal Jinn expression after desugaring — a record literal, a list, a map, a
nested record. The manifest is "parsed, never executed" (lamp.md §6.4) because
the *restriction lint* (§5) forbids the constructs that would execute (calls,
control flow, `*name`), not because the parser is a separate, weaker thing. The
same construct is available in ordinary `.jn` source for store schemas, build
config, and any declarative data — a net language win, stabilized provisionally
in `docs/stability.md`.

---

## 1. Grammar — decided, collision-free (Q3 resolved)

### 1.1 The block

A config block is an **indentation-scoped block of entries**, introduced by a
block head (a bare identifier in declaration position, or `is` after a binding
target). It reuses class-body indentation rules — no new lexer mode.

```
config-block   := block-head NEWLINE INDENT entry+ DEDENT
entry          := key ws clause-list NEWLINE
key            := IDENT
clause-list    := clause ("," clause)*
clause         := "is" value          # the binding clause
                | connector value      # a whitelisted connector clause
                | nested-block          # key introduces a sub-block
value          := literal | list | map | path | nested-record
list           := "[" value ("," value)* "]"
map            := "{" (key "is" value ("," key "is" value)*)? "}"
connector      := "requires" | "from" | "path" | "git" | "rev"
               | "opt" | "with" | "enables" | "visibility"  # closed, schema-driven
```

### 1.2 Why this does not collide with expression-statement parsing (the Q3 crux)

The danger prereqs Q3 raised: `release is opt 3, debug-info false` mixes `is`-
binding, bare keywords, and comma lists in ways that could parse as calls or
binary expressions. The resolution rests on **one structural rule**:

> Inside a config block, an entry is **always** `key clause-list` — the leading
> token of an entry is *always* a key (an identifier in binding position), and
> what follows is *always* a clause list, never an expression statement.

So:

- `opt 3` is **not** a call and **not** a binary expr. It is a **connector
  clause**: the connector keyword `opt` followed by its value `3`. Connectors are
  a *closed, schema-supplied set* (§1.1), so the parser knows `opt` introduces a
  one-value clause, not a function application. Outside a config block `opt` is an
  ordinary identifier; the connector reading is *only* active in config-block
  entry position.
- `release is opt 3, debug-info false` parses as the entry `release` with a
  clause-list of two clauses: `is opt 3` (the `is`-bound value is the connector-
  shaped record `{opt: 3}`) and `debug-info false` (the connector-less
  `key value` shorthand → `{debug-info: false}`). The comma separates *clauses of
  one entry*, never two entries (entries are newline-separated).
- A nested block is detected the same way class bodies are: a key followed by
  NEWLINE+INDENT introduces a sub-block instead of an inline clause-list.

The parser is therefore an LL(1) decision at entry start (`IDENT` → config
entry) and at clause start (`is` | connector-keyword | NEWLINE-INDENT). No
backtracking, no ambiguity with expression statements, because **config-block
entry position is a distinct grammatical context** the parser enters at the block
head and leaves at DEDENT.

### 1.3 The two shorthands, pinned

- **`key is value`** — the canonical binding. `version is '1.4.0'`.
- **`key value`** (connector-less) — sugar for `key is value` *only* when value
  is a single literal/list, used for one-word settings: `debug-info false`. The
  lint normalizes it to the `is` form.
- **`key connector value [, connector value]*`** — connector clauses build a
  record: `pomelo is '^0.7' with [tls]` → `{pomelo: {version: '^0.7', with:
  [tls]}}`; `postgres is requires ext/pq, enables 'metrics'` → `{postgres:
  {requires: ext/pq, enables: ['metrics']}}`.

---

## 2. Desugaring — to ordinary Jinn values

A config block desugars to a **nested record literal** (an object constructor).
The block head names the field; entries become fields; clause-lists become field
values (a record when there are connectors, a bare value otherwise); nested
blocks become nested records; repeated keys under a list-typed schema field
accumulate into a list.

```jinn
build
  toolchain is '^0.4'
  targets   is ['native', 'wasm32']
  profiles
    release is opt 3, debug-info false
    debug   is opt 0, debug-info true
```

desugars to:

```jinn
build is {
  toolchain is '^0.4',
  targets is ['native', 'wasm32'],
  profiles is {
    release is { opt is 3, debug-info is false },
    debug   is { opt is 0, debug-info is true },
  },
}
```

That right-hand side is **already valid Jinn** (record/object construction with
`is` fields, lists, maps). The config block is *purely* surface sugar over it.
There is exactly one evaluation model (Jinn value construction) and exactly one
type model (the record's inferred type, checked against the schema in §3).

---

## 3. Schemas

A config block is validated against a **schema** — a declared record type the
block must conform to. The manifest schema (`project`, `requires`, `provides`,
`build`, `capabilities`, `features`, `members`, `dev-requires`, `[target]`) is
the first schema, defined once in the compiler/lamp and used to:

- confirm every key is known (unknown key = precise diagnostic with the nearest
  valid key suggested);
- confirm each value's type (`opt` is an int, `debug-info` a bool, `targets` a
  list of target literals);
- supply the closed connector set valid in each position (so `with` is legal in a
  dependency clause but not in a `profiles` entry).

Schemas are how the *general* construct stays safe: the grammar is open (any
key/connector shape parses), the schema closes it per consumer. A store-schema
config block in ordinary `.jn` uses its own schema; the manifest uses the lamp
schema.

---

## 4. The manifest as the first consumer

`project.jn` is a file of top-level config blocks, each validated against the
lamp schema:

```jinn
project
  name    is 'orchard'
  version is '1.4.0'
  license is 'MIT'

requires
  json
  pomelo is '^0.7' with [tls]
  planner is path './planner'

provides
  lib orchard-core is 'src/core.jn'
  bin orchard      is 'src/main.jn', requires orchard-core

capabilities net.client, fs.read './assets'

features
  default  is ['json']
  postgres is requires ext/pq
  metrics  is requires ext/prom, enables 'metrics'
```

- `requires` is a block whose entries are dependency specs; a bare key (`json`)
  is the connector-less shorthand for "canonical package, default version
  range"; connectors (`from`, `path`, `git`, `rev`, `with`) refine it.
- `provides` entries lead with an output-kind connector (`lib`/`bin`) — these are
  schema-declared connectors, so `lib orchard-core is 'src/core.jn'` parses as the
  entry `orchard-core` of kind `lib` with body `is 'src/core.jn', requires
  orchard-core`.
- `capabilities` is the module-ceiling config block from `caps.md` §2.3 — it is a
  config block like any other, its entries the cap classes, its connectors the
  path scopes.
- `visibility` (`scope.md` §4) is a top-level manifest key declaring the
  package's own visibility ceiling (`public` (default) | `internal`); the
  resolver reads it on the *target* package, never on the consumer.

`name`/`version` desugar to record fields; the resolver reads the record, never
"runs" the manifest.

---

## 5. The restriction lint (lamp.md §2.1 / §6.4 made precise)

"Parsed, never executed" is enforced by a **validation pass over the parsed
block**, not by a weaker parser. Over the desugared record, the lint rejects —
with the precise diagnostics §2.1 promises — anything that is not pure
declarative data:

- **no calls** — a value that is a function application;
- **no control flow** — `if`/`for`/`while`/match in value position;
- **no `*name` definitions** — function definitions in the block;
- **no free identifiers** outside the schema's connector set and known package
  names (a bare identifier in value position must be a schema-permitted literal,
  e.g. a target name, not an arbitrary symbol reference).

Because the lint runs over the *already-typed* desugared record, the diagnostics
point at the exact source span. A manifest that tries to compute (`version is
git-describe()`) fails the lint at the call node, naming it. This closes npm's
single largest attack vector (`postinstall`) **by construction**: the manifest is
data, proven so by the lint, and lamp never has an execution path for it.

---

## 6. Generality and stabilization

- The construct is **general** (usable in ordinary `.jn`), but initially only the
  **manifest schema** is wired and supported. The general use (store schemas,
  build config) is documented in `docs/stability.md` as **provisional** until the
  schema mechanism and the lint surface are stabilized.
- This keeps the valuable, reusable construct in the language without committing
  to stabilize every corner before it has soaked. The manifest gets the full,
  supported experience now; general config blocks ride behind the provisional
  marker.

---

## 7. Cross-references

- `capabilities` block → `caps.md` §2.3.
- `requires`/`provides`/`members`/`visibility` semantics → `scope.md` §4, §6;
  lamp.md §2.2–§2.5.
- Restriction rationale (no install-time execution) → lamp.md §6.4.
- Provisional stabilization → `docs/stability.md`.
