# Jade Stores — Technical Specification

## Vision

A language-native persistence and analytics engine that unifies the best of relational databases (joins, constraints, ACID transactions), document stores (flexible schemas, nested data), columnar engines (vectorized analytics, compression), vector databases (semantic search, embeddings), graph databases (traversal, pathfinding), time-series databases (windowed aggregation, retention), and key-value stores (sub-microsecond lookups, TTL, pub/sub) — all as first-class compile-time features.

No ORM. No driver. No separate query language. No external daemon. The store IS the data structure, the query IS native code, and the wire format IS the memory layout.

**Core principles:**
1. **Whole-record I/O.** Records load and save as contiguous blobs — one `memcpy` per record, not per field. Dynamic fields (text, json) are indirected via pointers in the fixed-size record, with their heap data loaded in a second pass. Collections of records transfer as contiguous ranges.
2. **Compile-time everything.** Schema validation, index selection, join table generation, pack/unpack codegen, query planning, filter compilation — all at compile time. Zero runtime interpretation.
3. **One wire format.** JadePack — shared by disk persistence, actor mailboxes, remote procedure calls, and file export. The binary layout of a record in memory is the binary layout on disk and on the wire.
4. **Two APIs, one IR.** Method-based (`users.select(...)`) and query blocks (`query users where ...`) compile to identical native code. Choose whichever reads better.
5. **Vectorized execution.** Aggregations and analytical queries process data in batches (vectors) of contiguous typed values, enabling SIMD and cache-friendly access patterns. Inspired by DuckDB/MonetDB's columnar-vectorized model.
6. **Environment-aware migration.** Schema migrations are compiled into the application binary and execute at runtime with environment awareness — not at compile time. Migrations stack across environments (local → dev → stage → prod) and apply incrementally.

## Current State

- File format: `[8B magic "JADESTR\0"][8B count][8B record_size][records...]`
- String fields: fixed 256B buffers on disk (`[8B len][248B data]`)
- Operations: insert, count, query, delete, all, set (update), transaction
- Compound filters: AND/OR chaining
- Directory per store: `<name>/` in store directory with `<name>.store`, `<name>.schema`, `<name>.wal` files
- Query blocks: parser implemented (`where`, `sort`, `limit`, `take`, `skip`, `set`, `delete` clauses)

---

## Project-Level Configuration

### `project.jade`
```jade
store_path is 'data'              # directory for store directories (default: '.')
store_persistence is 'always'     # 'always' — persist on every mutation
                                  # 'explicit' — persist only when .save() is called
```

Only two knobs. Everything else is per-store or automatic.

---

## Store Types

### String vs Text
- **String** (`str`): max 256 bytes. Fixed-size buffer on disk (8B length prefix + 248B data). Fast comparisons, indexable, sortable. Used for names, emails, codes, identifiers — anything bounded.
- **Text** (`text`): variable-length. Stored via indirection — the fixed-size record contains a 16B pointer (8B offset + 8B length) to content in a separate `.text` heap file. Used for descriptions, bodies, logs — anything unbounded.

### JSON Fields
- **JSON** (`json`): variable-length, schema-flexible. Stored via indirection like `text` — 16B pointer in the fixed record, content in a `.json` heap file. Parsed on access, JadePack-encoded internally. Used for unstructured or semi-structured data where upfront schema is impractical.

### Numeric Types
All standard Jade numeric types are valid store fields: `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32`, `f64`, `bool`.

### Enum Fields
Store fields can be enum types. Stored as their discriminant tag (i32) on disk. Variants with payloads are serialized via JadePack.

```jade
enum Status
    Active
    Inactive
    Suspended str  # reason

store Event
    name as str
    status as Status
```

---

## Record Memory Model

A store record in memory is a **contiguous fixed-size struct** with indirection pointers for variable-length fields. Loading a record is a single `memcpy` of the fixed portion, followed by resolution of any dynamic pointers.

### Fixed-Size Records (no text/json fields)

```
┌────────────────────────────────────────────────────────────┐
│ sid(8) │ uuid(256) │ hash(256) │ created(8) │ updated(8) │ deleted(8) │ first_name(256) │ last_name(256) │ age(8) │
└────────────────────────────────────────────────────────────┘
```

One `memcpy` from disk/mmap to memory. One `memcpy` back for writes. The in-memory layout IS the on-disk layout IS the JadePack layout. Zero serialization.

### Records with Dynamic Fields (text/json)

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ...fixed fields... │ bio_offset(8) │ bio_len(8) │ meta_offset(8) │ meta_len(8) │
└──────────────────────────────────────────────────────────────────────────┘
                           │                             │
                           ▼                             ▼
                    ┌──────────────┐              ┌──────────────┐
                    │ text heap    │              │ json heap    │
                    │ (bio data)   │              │ (meta data)  │
                    └──────────────┘              └──────────────┘
```

Loading sequence:
1. `memcpy` the fixed-size record (one I/O)
2. For each dynamic field: read from heap file at `(offset, len)` → heap-allocate and attach pointer
3. The Jade-side struct has the dynamic fields as pointers — access is transparent

Saving sequence:
1. Append dynamic field data to heap file, get `(offset, len)`
2. Write fixed record with updated offsets (one I/O)

### Bulk Operations — Contiguous Range I/O

Loading N records: `memcpy(dest, src, N × record_size)` — one operation for the entire range. Dynamic fields resolved in a second batch pass.

```jade
# Under the hood:
# users.all()       → mmap entire .store file, return pointer + count
# users.select(...) → scan with compiled predicate, collect matching offsets, batch-read
# users.paginate(N) → read N × record_size bytes from current cursor offset
```

For `@mem` stores, records ARE the memory — no copy at all. The store IS an array of structs in the process address space. Queries iterate the array directly.

---

## Store Declaration

### Basic Syntax
```jade
store User
    name as str
    email as str
    age as i64
    bio as text
```

### Full Syntax with Decorators
```jade
store User @mem
    first_name as str
    last_name as str @index
    email as str @index @unique
    age as i64 @sorted
    bio as text
    metadata as json
    score as f64 @transient
    rank as i64 @increment
    &address as Address          # has-one relation
    &messages as [Message]       # has-many relation
```

### Specialized Store Types

```jade
store Embedding @vector(768)     # vector store, 768 dimensions
    label as str
    source as str

store Friendship @graph           # graph store
    &from as Person
    &to as Person
    kind as str
    weight as f64

store SensorReading @timeseries(timestamp)   # time-series store
    timestamp as i64
    device_id as str @index
    temperature as f64
    humidity as f64

store Cache @kv                   # key-value store, Redis-like
    # no fields — keys are strings, values are any serializable type

store Session @kv @mem            # in-memory KV with persistence
```

See **Specialized Store Types** section below for full details.

---

## Store Decorators

### Store-Level Decorators

| Decorator | Effect |
|-----------|--------|
| `@simple` | No built-in fields (sid, uuid, hash, created, updated, deleted are omitted). Raw record only. |
| `@mem` | All records kept in memory. Background persistence to disk. |
| `@transient` | In-memory only. Never persisted. Lost on restart. |
| `@versioned` | Record-level versioning. Mutations create new record versions with the previous version retained. Enables `.history()`, `.at_version()`, `.rollback()` per record. |
| `@vector(N)` | Vector store with N-dimensional embeddings. Enables similarity search, nearest-neighbor queries. |
| `@graph` | Graph store. Records are edges with `&from` and `&to` relationships. Enables traversal, pathfinding, subgraph queries. |
| `@timeseries(field)` | Time-series store keyed on the named timestamp field. Enables windowed aggregation, downsampling, retention policies. |
| `@kv` | Key-value store. String keys, any serializable value. In-memory with persistent backend. Replaces Redis/Consul/memcached. TTL, pub/sub, atomic ops. |

### Field-Level Decorators

| Decorator | Effect |
|-----------|--------|
| `@index` | Hash index on this field. O(1) equality lookups. Separate `.idx` file. |
| `@unique` | Uniqueness constraint. Insert/update rejected on duplicate. Implies `@index`. |
| `@sorted` | B-tree index. Enables range queries, ordered iteration, binary search. |
| `@transient` | Not persisted to disk. Held in memory only. Resets on restart. |
| `@increment` | Auto-incrementing i64. Value assigned on insert, monotonically increasing. |
| `@required` | Field must be provided on insert. No zero-value default. Compile-time enforced. |
| `@versioned` | Field history retained. Previous values accessible by version number. |
| `@default(value)` | Default value when not provided on insert. Compile-time constant. |

### `@versioned` — Record-Level vs Field-Level

**Field-level `@versioned`:** Individual fields retain history. The record holds the latest value; previous values are stored in an append-only version segment. Accessing `doc.body.history()` returns the version chain for that field only.

**Store-level `@versioned` (record versioning):** When any field of a record is mutated, the entire previous record is preserved as a version. The store maintains a version chain per record — not per store. This is editing history for records, like revision history for a wiki page or git commits for a file.

```jade
store Post @versioned
    title as str
    body as text
    author as str

post is posts.insert(title is 'Draft', body is 'Hello', author is 'Alice')
post.title is 'Final'               # record version 2 created, version 1 preserved
post.body is 'Hello world'          # record version 3 created

# Per-record version history
post.history()                       # [v1: {title: 'Draft', body: 'Hello'}, v2: {title: 'Final', body: 'Hello'}, v3: ...]
post.at_version(1)                   # returns the record as it was at v1
post.rollback(1)                     # restores record to v1 state (creates v4)
post.diff(1, 3)                      # field-by-field diff between versions

# Query historical versions
posts.history(post.sid)              # all versions of record with this sid
posts.at_version(post.sid, 1)       # specific version of specific record
```

**Combined (`@versioned` store + `@versioned` field):** The store-level `@versioned` tracks whole-record snapshots. The field-level `@versioned` additionally tracks per-field granular history with diff support. When both are present, you get whole-record snapshots AND fine-grained field-level change tracking.

**Implementation:** Record versions are stored in a separate append-only file (`<store>.versions`). Each entry is `[8B sid][8B version_num][8B timestamp][full record bytes]`. The current record in the main `.store` file always holds the latest version. The version file is indexed by `(sid, version)` for O(log n) lookups.

**Compaction:** `posts.compact(keep is 10)` — retains only the latest N versions per record. `posts.compact(older_than is 90)` — prune versions older than N days.

---

## Built-In Fields

Every store (unless `@simple`) automatically includes:

| Field | Type | Description |
|-------|------|-------------|
| `sid` | i64 | Sequential ID. Monotonic, gap-free within a session. Assigned on insert. |
| `uuid` | str | UUID v4. 36-char string. Generated on insert. Globally unique. |
| `hash` | str | BLAKE3 hash of the record's user-defined fields. Content-addressable. |
| `created` | i64 | Unix timestamp (seconds). Set on insert. Immutable. |
| `updated` | i64 | Unix timestamp (seconds). Set on every mutation. |
| `deleted` | i64 | Unix timestamp (seconds). Set on soft-delete. 0 if not deleted. |

This means every record is:
- **Identifiable** — by sequential ID, UUID, or content hash
- **Temporal** — created/updated/deleted timestamps for audit trails
- **Soft-deletable** — `deleted` field enables trash/restore without data loss

### Soft Delete Semantics
- `users.delete(user)` — sets `deleted` to current timestamp
- `users.destroy(user)` — hard-deletes, removes record from file
- Queries automatically filter out soft-deleted records
- `users.include_deleted()` — query modifier to include soft-deleted records
- `users.restore(user)` — clears the `deleted` timestamp
- `hash` field enables deduplication: insert-if-not-exists by content hash

---

## Store API — Method-Based Operations

All store operations are methods on the store object. No reserved keywords for insert/delete/select. The store is a first-class value with a typed method interface generated by the compiler.

### Insert

```jade
# Positional — fields in declaration order
users.insert('Alice', 'Smith', 'alice@test.com', 30)

# Named — any order, self-documenting
users.insert(first_name is 'Alice', last_name is 'Smith', email is 'alice@test.com', age is 30)

# From a constructed record
u is User(first_name is 'Alice', last_name is 'Smith', email is 'alice@test.com', age is 30)
users.insert(u)

# Returns the inserted record (with sid, uuid, hash, timestamps populated)
alice is users.insert(first_name is 'Alice', last_name is 'Smith', email is 'alice@test.com', age is 30)
log alice.sid       # 1
log alice.uuid      # 'a1b2c3d4-...'
log alice.created   # 1712400000
```

`insert` always returns the fully-populated record. Built-in fields (sid, uuid, hash, created, updated) are filled automatically. `@increment` fields are assigned. `@unique` constraints are checked — insert fails with an error if violated.

### Delete (soft) and Destroy (hard)

```jade
# By record — soft-deletes (sets deleted timestamp)
users.delete(alice)

# By sid
users.delete(1)

# By uuid
users.delete('a1b2c3d4-...')

# By filter — deletes all matching
users.delete(last_name is 'Smith')

# Record method — soft-delete self
alice.delete()

# Hard delete — permanently removes from disk
users.destroy(alice)
users.destroy(1)
alice.destroy()

# Restore a soft-deleted record
users.restore(alice)
alice.restore()
```

When `@cascade` is on a relation, delete/destroy propagates to related records.

### Update — Direct Field Mutation

```jade
# Mutate a field directly on the record
alice.age is 31

# When store_persistence is 'always': this write is persisted immediately
# When store_persistence is 'explicit': mutation held in memory until save

# Explicit save (required when store_persistence is 'explicit')
alice.save()

# Save through the store
users.save(alice)

# Bulk save (flushes all dirty records for 'explicit' mode)
users.save()
```

There is no separate `update` keyword. Mutating a store record's field IS the update. The compiler knows the variable holds a store record and intercepts the assignment to:
1. Update the in-memory state
2. Update the `updated` timestamp
3. Recompute the `hash`
4. If `store_persistence is 'always'`: write to WAL and flush
5. If `store_persistence is 'explicit'`: mark record dirty, defer write

This makes store records feel like regular structs. The persistence is transparent.

### Batch Update — `.set()`

For updating multiple fields at once, `.set()` avoids multiple individual writes (and multiple WAL entries / timestamp updates / hash recomputations):

```jade
# Named arguments — update multiple fields in one operation
alice.set(age is 31, last_name is 'Jones')

# From a map
updates is {age: 31, last_name: 'Jones'}
alice.set(updates)

# On the store with a lookup — update by sid or uuid
users.set(1, age is 31, last_name is 'Jones')
users.set('a1b2c3d4-...', last_name is 'Jones')

# Bulk set — update all matching records
users.set(age gt 65, status is 'retired')
```

Semantics:
- A single `.set()` call produces **one** WAL entry, **one** `updated` timestamp, **one** `hash` recomputation
- For `store_persistence is 'always'`: one disk write, not N writes for N fields
- For `@versioned` stores: one store version increment, not N
- For `@versioned` fields: each versioned field in the set gets one new version entry
- Compile-time validated: unknown field names or type mismatches are compile errors
- `.set()` with a map is runtime-checked for key validity — unknown keys are ignored with a warning (debug builds) or silently skipped (release builds)

### Select / Query

```jade
# By sid — returns single record
alice is users.get(1)

# By uuid — returns single record
alice is users.get('a1b2c3d4-...')

# Filter — returns Vec of matching records
smiths is users.select(last_name is 'Smith')

# Multiple conditions (AND)
young_smiths is users.select(last_name is 'Smith', age lt 30)

# Complex filters with comparison operators
results is users.select(age gte 18, age lt 65, last_name neq 'Bot')

# With limit, skip, sort
page is users.select(age gt 18, limit is 10, skip is 20, sort is 'age')
page is users.select(age gt 18, limit is 10, skip is 20, sort is '-age')  # descending

# First matching record (returns single record or Nothing)
first is users.first(last_name is 'Smith')

# All records
everyone is users.all()

# Count
total is users.count()
active is users.count(age gt 18)

# Exists check
has_alice is users.exists(email is 'alice@test.com')

# Distinct values for a field
names is users.distinct(last_name)

# Include soft-deleted records
all_including_deleted is users.include_deleted().select(last_name is 'Smith')
```

### Complex Filters — OR, Grouping, Nesting

Multiple arguments to `.select()` are AND'd. For OR and complex boolean logic, use `or()` groups:

```jade
# Simple OR — any condition matches
results is users.select(or(age lt 18, age gt 65))

# AND + OR — senior Smiths or anyone under 18
results is users.select(last_name is 'Smith', or(age lt 18, age gt 65))

# Nested groups — (Smith AND age > 30) OR (Jones AND age > 25)
results is users.select(
    or(
        and(last_name is 'Smith', age gt 30),
        and(last_name is 'Jones', age gt 25)
    )
)

# OR across relations — users in Oregon or with unread messages
results is users.select(
    or(
        address.state is 'OR',
        messages.any(read is false)
    )
)

# NOT — negate a condition
results is users.select(not(last_name is 'Smith'))
results is users.select(age gt 18, not(status is 'banned'))

# IN — match against a set of values
results is users.select(status in ['active', 'pending'])
results is users.select(age in [21, 25, 30])

# BETWEEN — range shorthand
results is users.select(age between [18, 65])

# LIKE — pattern matching on strings
results is users.select(email like '%@test.com')
results is users.select(last_name like 'Sm%')
```

`or()`, `and()`, `not()`, `in`, `between`, and `like` are compile-time constructs — the compiler translates them directly to native predicate code. They compose freely: `or(and(...), not(...))` nests arbitrarily. The optimizer flattens redundant nesting.

### Paginator / Cursor

For large result sets, a paginator provides batched iteration without loading all records into memory:

```jade
# Create a paginator — 50 records per page
pages is users.paginate(50)

# Advance through pages
first_page is pages.next()       # Vec of up to 50 records
second_page is pages.next()      # next 50
third_page is pages.next()       # next 50 (or fewer if near end)

# Check if more pages exist
if pages.has_next()
    more is pages.next()

# With filters — paginate over a subset
active_pages is users.paginate(25, age gt 18, sort is 'last_name')

# With cursor — paginate through filtered + sorted results
pages is users.paginate(100, status is 'active', sort is 'created')
while pages.has_next()
    batch is pages.next()
    for user in batch
        process(user)

# Reset to beginning
pages.reset()

# Get total count (without loading all records)
log pages.total()     # total matching records
log pages.page_count() # total number of pages

# Jump to a specific page
page_5 is pages.page(5)
```

Paginator internals:
- Backed by a **cursor** — an opaque position in the result set (record offset + filter state)
- For indexed/sorted queries: cursor is a B-tree position — advancing is O(log n) seek + O(page_size) scan
- For unindexed queries: cursor is a record offset — sequential scan with compiled predicate
- The paginator holds a read snapshot — concurrent writes do not affect in-progress pagination (snapshot isolation)
- Memory: only one page of records is in memory at a time (previous pages are released)
- Paginators are first-class values — pass them to functions, store them in variables, send them across actors (the cursor state is serializable via JadePack)

### Filter Operators

Inside `.select()`, `.count()`, `.first()`, `.exists()`, `.paginate()`, and other query methods:

| Syntax | Meaning |
|--------|---------|
| `field is value` | Equals |
| `field neq value` | Not equals |
| `field lt value` | Less than |
| `field gt value` | Greater than |
| `field lte value` | Less than or equal |
| `field gte value` | Greater than or equal |
| `field in [values]` | Set membership |
| `field between [lo, hi]` | Range (inclusive) |
| `field like 'pattern'` | String pattern match (`%` = wildcard) |
| `or(...)` | OR group |
| `and(...)` | AND group (explicit nesting) |
| `not(...)` | Negation |
| `limit is N` | Max results |
| `skip is N` | Skip first N |
| `sort is 'field'` | Sort ascending |
| `sort is '-field'` | Sort descending |

Multiple bare filters are AND'd. All filters compile to native code — index lookups when the field has `@index` or `@sorted`, sequential scan otherwise.

### Aggregation

```jade
avg_age is users.avg(age)
total is orders.sum(total)
highest is scores.max(score)
lowest is scores.min(score)

# With filter
avg_active is users.avg(age, active is true)

# Grouped
by_status is orders.group(status).sum(total)
by_cat is products.group(category).avg(price)
by_city is users.group(address.city).count()
```

Aggregation methods operate directly on column data when available (`@mem` stores or column files). SIMD-vectorizable for contiguous numeric arrays.

### Projection

```jade
# Select specific fields — returns lightweight records with only those fields
emails is users.pluck(email)
names_and_ages is users.pluck(first_name, last_name, age, age gt 18)
```

`pluck` returns records with only the requested fields. For column stores, this reads only the relevant column files — no wasted I/O.

---

## Query Blocks — Inline SQL-Like Syntax

Alongside the method-based API, Jade provides query blocks — an indentation-based query syntax that reads like SQL but compiles to the same native code. Query blocks are expressions that return results.

### Basic Syntax

```jade
# Select with filter, sort, limit
result is query users
    where age gt 21
    sort name
    limit 10

# Equivalent method call:
result is users.select(age gt 21, sort is 'name', limit is 10)
```

### All Clauses

```jade
result is query users
    where age gt 21 and last_name is 'Smith'
    where email like '%@test.com'          # multiple where clauses AND together
    sort -created                           # descending sort
    sort last_name                          # secondary sort (ascending)
    skip 20
    limit 10

# Complex OR in query blocks
result is query users
    where age lt 18 or age gt 65
    where last_name is 'Smith' or last_name is 'Jones'
    sort age
    limit 100
```

### Query Block as Update

```jade
# Set (update) via query block
query users
    where last_name is 'Smith'
    set age is 31
    set status is 'active'

# Delete via query block
query users
    where age lt 0
    delete
```

### Query Block with Relations

```jade
# Filter through relations
local is query users
    where address.state is 'OR'
    sort last_name

# Nested relation filter
high_value is query orders
    where customer.age gt 30
    where total gt 100.0
    sort -total
    limit 50
```

### Query Block Aggregation

```jade
avg is query users
    where age gt 18
    avg age

totals is query orders
    group status
    sum total

breakdown is query users
    where active is true
    group address.state
    count
```

### Query Blocks vs Method API

Both APIs compile to identical native code. The choice is stylistic:

| Method API | Query Block |
|------------|-------------|
| `users.select(age gt 21, sort is 'name', limit is 10)` | `query users` / `where age gt 21` / `sort name` / `limit 10` |
| Inline, composable, good for pipelines | Multi-line, readable, good for complex queries |
| Programmatic — result is a function call | Declarative — reads like a query language |
| Filters are arguments | Clauses are indented lines |

Query blocks are parsed by `parse_query_block` (7 clause types: `where`, `sort`, `limit`, `take`, `skip`, `set`, `delete`). The compiler lowers both APIs to the same IR — store scan + predicate + sort + limit. No performance difference.

---

## Relationships

### Syntax

The `&` prefix on a field name declares a relationship to another store. The compiler resolves the target store by type name, generates a join table (`.rel` file), and auto-joins on query.

```jade
store Address
    street as str
    city as str
    state as str
    zip as str

store Message
    subject as str
    body as text
    read as bool

store User @mem
    first_name as str
    last_name as str
    email as str @index @unique
    &address as Address          # has-one: User has one Address
    &messages as [Message]       # has-many: User has many Messages
```

### What `&` generates

**`&address as Address`** (has-one):
- Generates a join table `.rel` file mapping user sid → address sid
- On query, automatically joins the Address record: `user.address.city` works directly
- On insert, accepts either an Address sid or an inline Address record (auto-inserted into the Address store)
- If `@cascade`: deleting the User also deletes the Address

**`&messages as [Message]`** (has-many):
- Generates a join table `.rel` file mapping user sid → [message sids]
- On query, automatically resolves all Messages for the User: `user.messages` is a Vec
- Eager by default — loaded with the parent record
- If `@lazy`: resolved on first access instead

### Relationship Decorators

```jade
store User
    &address as Address @cascade
    &messages as [Message] @lazy
    &manager as User @required
    &tags as [Tag] @cascade @lazy
```

| Decorator | Effect |
|-----------|--------|
| `@cascade` | Delete/destroy propagates to related records. |
| `@lazy` | Related records loaded on first access, not with parent. |
| `@required` | Relationship must be set on insert. Compile-time enforced. |

### Foreign Key Resolution

The compiler resolves relationships at compile time:

1. `&address as Address` — generates `address.rel` in `data/users/rels/` and `user.rel` in `data/addresses/rels/`
2. `&messages as [Message]` — generates `messages.rel` in `data/users/rels/` and `user.rel` in `data/messages/rels/`
3. Self-referential: `&manager as User` — generates `manager.rel` in `data/users/rels/`
4. All `.rel` files are automatically indexed for O(1) lookups

### Relation Access

```jade
# Relations are accessed as fields — joins happen automatically
alice is users.first(email is 'alice@test.com')
log alice.address.city                  # joins Address (eager or lazy depending on decorator)
log alice.messages                      # resolves Vec of Messages

# Filter through relations
local is users.select(address.state is 'OR')

# Deep traversal
c is customers.first(name is 'Alice')
for order in c.orders
    for item in order.items
        log '{item.product_name}: {item.quantity}'
```

### Insert with Relations

```jade
# Insert the related record first, then reference by sid
addr is addresses.insert(street is '123 Main St', city is 'Portland', state is 'OR', zip is '97201')
alice is users.insert(first_name is 'Alice', last_name is 'Smith', email is 'alice@test.com', age is 30, address is addr)

# Or inline — the address is auto-inserted into the Address store
alice is users.insert(
    first_name is 'Alice',
    last_name is 'Smith',
    email is 'alice@test.com',
    age is 30,
    address is Address(street is '123 Main St', city is 'Portland', state is 'OR', zip is '97201')
)

# Add to has-many after creation
msg is messages.insert(subject is 'Hello', body is 'Welcome!', read is false)
alice.messages.add(msg)

# Remove from has-many (does not delete the message, just unlinks)
alice.messages.remove(msg)

# Replace has-one
new_addr is addresses.insert(street is '456 Oak Ave', city is 'Seattle', state is 'WA', zip is '98101')
alice.address is new_addr
```

### Many-to-Many

Many-to-many relationships use an explicit junction store:

```jade
store Student
    name as str
    &enrollments as [Enrollment]

store Course
    title as str
    &enrollments as [Enrollment]

store Enrollment @simple
    &student as Student
    &course as Course
    grade as f64
    enrolled_at as i64
```

The compiler sees both `Student` and `Course` referencing `Enrollment`, and `Enrollment` referencing both back. It generates the join tables and enables traversal in both directions:

```jade
s is students.first(name is 'Alice')
for e in s.enrollments
    log '{e.course.title}: {e.grade}'

c is courses.first(title is 'Compilers')
for e in c.enrollments
    log '{e.student.name}: {e.grade}'
```

---

## JadePack — Universal Wire Format

JadePack is Jade's zero-overhead binary serialization format. It is the canonical encoding for:
- **Store records** on disk
- **Actor messages** (local and remote)
- **Remote procedure calls** (future distributed actors)
- **File export/import** (data interchange)
- **Snapshot/checkpoint** data

### Design Principles
1. **Compiler-generated**: the compiler emits `pack()` and `unpack()` functions for every type. No reflection, no schema registry at runtime.
2. **Zero-copy reads**: fixed-size fields are read directly from the buffer with pointer arithmetic. No parsing, no allocation.
3. **Deterministic layout**: the same type always produces the same byte layout. Byte-for-byte reproducible. Enables content-addressable hashing.
4. **Self-describing optional**: a compact type tag can be prepended for dynamic contexts (heterogeneous channels, debugger inspection). Omitted when both sides know the schema at compile time.
5. **Platform-independent**: little-endian, explicit sizes. Portable across architectures.

### Encoding Rules

| Type | Wire Format | Size |
|------|-------------|------|
| `bool` | 1 byte (0x00 or 0x01) | 1B |
| `i8`/`u8` | 1 byte | 1B |
| `i16`/`u16` | 2 bytes LE | 2B |
| `i32`/`u32` | 4 bytes LE | 4B |
| `i64`/`u64` | 8 bytes LE | 8B |
| `f32` | 4 bytes LE (IEEE 754) | 4B |
| `f64` | 8 bytes LE (IEEE 754) | 8B |
| `str` | `[8B len][248B data]` (fixed 256B) | 256B |
| `text` | `[8B len][data...]` (variable) | 8B + len |
| `enum` | `[4B tag][payload...]` | 4B + payload |
| `Option` | `[1B tag][value if present]` | 1B + value |
| `Vec` / `[T]` | `[8B count][elements...]` | 8B + count × sizeof(T) |
| `Map` | `[8B count][key,value pairs...]` | 8B + count × (sizeof(K) + sizeof(V)) |
| struct / type | fields concatenated in declaration order | sum of field sizes |

Relations are NOT encoded in JadePack — they are resolved via join tables. A packed record contains only its own fields. This keeps the wire format flat, fixed-size for fixed-schema records, and free of recursive serialization.

### Compile-Time Codegen

For every type used in a store or actor message, the compiler generates:

```
# Pseudocode — what the compiler emits as LLVM IR

*pack_User buf as %i8, u as User
    memcpy buf, %u.first_name, 256       # fixed str
    memcpy buf + 256, %u.last_name, 256
    memcpy buf + 512, %u.email, 256
    store i64 u.age at buf + 768         # i64 field

*unpack_User buf as %i8 returns User
    u as User
    u.first_name is unpack_str(buf)
    u.last_name is unpack_str(buf + 256)
    u.email is unpack_str(buf + 512)
    u.age is load_i64(buf + 768)
    return u
```

No vtable. No type tag lookup. No allocation for fixed-size records. Direct `memcpy` in, direct pointer arithmetic out.

### Shared with Actor Messages

```jade
actor OrderProcessor
    @process_order order as Order
        # `order` deserialized from JadePack automatically
        log 'processing {order.customer.name}'

*main
    proc is spawn OrderProcessor
    o is orders.get(42)
    send proc, @process_order(o)   # `o` serialized to JadePack into mailbox
```

For local actors (same process), messages that fit in the mailbox ring buffer are copied directly — the pack/unpack is a `memcpy`. For remote actors (future), the same JadePack bytes go over the network with no additional serialization step.

### Shared with Remote Processing (Future)

```jade
# Future: distributed actors
remote_proc is connect OrderProcessor at 'worker-2:9090'
send remote_proc, @process_order(o)   # same JadePack, sent over TCP
```

The wire format is identical whether the actor is local or remote. The compiler generates the same `pack()` / `unpack()` functions. The only difference is the transport: `memcpy` into a ring buffer vs. `send()` into a socket.

### Export / Import

```jade
# Serialize a store to a portable binary file
users.export('users_backup.jpack')

# Import from a JadePack file
users.import('users_backup.jpack')

# Serialize a single record
bytes is pack(alice)          # returns Vec of u8
alice2 is unpack User bytes   # returns User
```

JadePack files have a header: `[8B magic "JADEPACK"][4B version][4B schema_hash][8B count][records...]`. The `schema_hash` is a compile-time hash of the schema — the compiler checks it on import and rejects incompatible schemas.

---

## Dual Storage Engine

### Row Store (default)
Optimized for:
- Point queries (fetch one record by ID or indexed field)
- Inserts and updates (append or overwrite in place)
- Transactions (record-level locking)
- OLTP workloads

File layout:
```
[8B magic][4B version][4B schema_hash][8B count][8B record_size][8B flags][records...]
```

Flags byte encodes: has-WAL, has-column-store, @mem, @versioned, @simple.

### Column Store (opt-in per field or per store)
Each field stored in a separate file (`<field>.col`), optimized for:
- Aggregation (sum, avg, min, max over millions of rows)
- Analytics (scan one column without reading entire records)
- Compression (same-type data compresses well with RLE, delta, dictionary encoding)

The compiler maintains BOTH row and column representations for `@mem` stores. Writes go to the row store and column files are rebuilt in the background. For non-`@mem` stores, column files are maintained incrementally on write.

This is the "best of all worlds" approach: point queries and mutations hit the row store, analytics hit the column store, and the programmer never has to choose.

---

## In-Memory Layer (`@mem`)

When a store is marked `@mem`:

1. **On program start**: entire store is loaded into memory (mmap'd or deserialized via JadePack)
2. **Reads**: served from memory. Zero I/O. Zero syscalls.
3. **Writes**: applied to memory immediately, queued for background persistence
4. **Background flush**: a dedicated coroutine writes dirty pages to disk
   - Flush on graceful shutdown
   - WAL ensures crash safety between flushes
5. **Memory pressure**: LRU eviction for stores that exceed a configurable memory budget

For non-`@mem` stores, reads still go through an OS page cache (via mmap), so hot data is effectively cached by the kernel.

### Relation Caching
When a `@mem` store has relations without `@lazy`, the related records are loaded into memory alongside the parent. The in-memory representation is a full object graph — no join table lookups until cache miss.

---

## Indexing

### Hash Index (`@index`)
- Separate `<field>.idx` file in the store's `idx/` directory
- Open-addressing hash table, power-of-2 sizing
- Stores `(hash, record_offset)` pairs
- Rebuild from store file on cold start (or from WAL replay)
- Resize (double) when load factor exceeds 0.7

### B-Tree Index (`@sorted`)
- Separate `<field>.btree` file in the store's `idx/` directory
- B+ tree with 4KB page size
- Leaf nodes store `(key, record_offset)` pairs
- Supports: range queries, ordered iteration, min/max in O(log n)
- Bulk-load optimization for initial index creation

### Bloom Filter (automatic)
- For non-indexed fields queried more than once (tracked at compile time)
- Compiler emits a bloom filter check before full scan
- False positive rate ~1% with 10 bits/element
- Rebuilt on startup, updated on insert

### Composite Index
- `@index(name, age)` — multi-field hash index
- `@sorted(category, created)` — multi-field B-tree for compound range queries

### Relation Index (automatic)
- Every `.rel` join table file is itself an indexed structure
- Bidirectional: parent→child and child→parent lookups are both O(1)
- The compiler never generates a relationship without an index

### Full-Text Index (`@search`) — future
- Inverted index for text fields
- `users.search(bio is 'compiler engineer')`
- Trigram-based for prefix/substring matching

---

## Write-Ahead Log (WAL)

Every persistent store gets a `<name>.wal` file:

```
[8B magic "JADEWAL\0"][entries...]
Entry: [4B len][1B op][8B timestamp][payload bytes][4B CRC32]
```

- **Insert**: op=1, payload=full record (JadePack encoded)
- **Update**: op=2, payload=record offset + new record (JadePack encoded)
- **Delete**: op=3, payload=record offset + delete timestamp
- **Destroy**: op=4, payload=record offset

On startup:
1. Check WAL for uncommitted entries
2. Replay forward from last checkpoint
3. Truncate WAL

Checkpoint triggers:
- WAL exceeds 16MB
- On graceful shutdown
- On explicit `.save()` call

With `store_persistence is 'explicit'`, mutations accumulate in the WAL until `.save()` is called, which triggers a checkpoint.

---

## Query Compilation

Queries are not interpreted — they compile to native loops, index lookups, and SIMD comparisons. Every `.select()`, `.first()`, `.count()` call is compiled to a function that directly reads store data.

### Index Selection
The compiler analyzes filter arguments at compile time:
1. If a filter field has `@index` → hash lookup (O(1))
2. If a filter field has `@sorted` → B-tree range scan (O(log n))
3. If a filter field has a bloom filter → bloom check, then scan if positive
4. Otherwise → sequential scan with compiled predicate

### Join Strategy (for explicit joins and relation filters)
- **Index lookup**: for `&` relationships — always O(1) via `.rel` join table (default)
- **Hash join**: build hash table on smaller store, probe with larger
- **Merge join**: when both sides have `@sorted` on the join key
- **Nested loop**: fallback for small stores (< 1000 records)

### Aggregation Optimization
- `@mem` stores: aggregation runs on in-memory arrays — no I/O
- Column store fields: contiguous type-homogeneous arrays — SIMD-vectorizable (sum/min/max on packed i64/f64)
- `@sorted` fields: min/max in O(1) — read first/last B-tree leaf

---

## Computed Fields and Store Methods

### `@transient` Fields
Not persisted. Held in memory only, reset on restart. Useful for caches, session state, counters.

```jade
store User
    first_name as str
    last_name as str
    views as i64 @transient   # in-memory only, resets on restart
```

### Store Methods
Methods defined on a store type act as computed properties:

```jade
store User
    first_name as str
    last_name as str
    &orders as [Order]

    *full_name returns str
        '{first_name} {last_name}'

    *total_spent returns f64
        orders.sum(total)
```

`user.full_name` is computed on access. `user.total_spent` triggers aggregation on the related orders. Methods are zero-cost — compiled to inline native code.

---

## Transactions

```jade
transaction
    alice is users.insert(first_name is 'Alice', last_name is 'Smith', email is 'alice@test.com', age is 30)
    orders.insert(total is 49.99, status is 'pending', customer is alice)
    alice.age is 31
```

Transaction semantics:
- All-or-nothing: if any operation fails, all are rolled back
- WAL-backed: operations written to WAL, committed atomically
- Isolation: serializable within a single process (file lock for multi-process)
- Nested transactions: savepoints
- Cascading deletes within a transaction are atomic

---

## Versioned Fields and Stores

This section covers the detailed API for `@versioned`. See **Store Decorators** above for the conceptual overview.

### Field-Level `@versioned`

```jade
store Document
    title as str
    body as text @versioned
    author as str
```

Fields marked `@versioned` retain history. Updates create a new version instead of overwriting:

```jade
doc is documents.insert(title is 'Draft', body is 'Hello world', author is 'Alice')
doc.body is 'Hello updated world'

# Access version history for the versioned field
history is doc.body.history()
for ver in history
    log 'v{ver.version}: {ver.value}'

# Access specific version
old_body is doc.body.at_version(1)

# Diff between versions
changes is doc.body.diff(1, 2)
```

Implementation: versioned fields are stored in a separate append-only segment (`<store>.<field>.versions`). Each entry is `[8B sid][8B version_num][8B timestamp][value bytes]`. The main record holds the latest value. Version numbers are per-record, not global.

Compaction: `documents.compact(keep is 10)` — retains only the latest N versions per record.

### Store-Level `@versioned` — Record Versioning

Store-level `@versioned` means **record versioning**: when any field of a record is mutated, the previous version of the entire record is preserved. The store maintains a version chain **per record** — not per store. Think of it as revision history for a wiki page, or git commits for a file.

```jade
store Post @versioned
    title as str
    body as text
    author as str

post is posts.insert(title is 'Draft', body is 'Hello', author is 'Alice')
post.title is 'Final'               # record version 2 created, version 1 preserved
post.body is 'Hello world'          # record version 3 created

# Per-record version history
post.history()                       # [v1: {title: 'Draft', ...}, v2: {title: 'Final', ...}, v3: ...]
post.at_version(1)                   # returns the record as it was at v1
post.rollback(1)                     # restores record to v1 state (creates v4)
post.diff(1, 3)                      # field-by-field diff between versions

# Query historical versions through the store
posts.history(post.sid)              # all versions of record with this sid
posts.at_version(post.sid, 1)       # specific version of specific record

# Count versions
post.version_count()                 # 4 (v1, v2, v3, v4)
posts.version_count(post.sid)        # same thing through the store
```

Implementation:
- Record versions are stored in a separate append-only file (`<store>.versions`)
- Each entry is `[8B sid][8B version_num][8B timestamp][full record bytes]`
- The current record in the main `.store` file always holds the latest version
- The version file is indexed by `(sid, version)` for O(log n) lookups via a B-tree on `(sid, version_num)`
- Insert creates version 1. Each subsequent mutation creates version N+1. The version chain is per-record.
- `rollback(N)` does NOT destroy versions — it creates a new version whose content equals version N

**This is NOT store-level snapshotting.** There is no concept of "store version 1, 2, 3" where each is a snapshot of every record. Each record has its own independent version chain. Two records in the same `@versioned` store can be at different version numbers.

Compaction:
```jade
posts.compact(keep is 10)            # keep latest 10 versions per record
posts.compact(older_than is 90)      # prune versions older than 90 days
posts.compact(sid is post.sid, keep is 5)  # compact a specific record's history
```

### Combined: `@versioned` Store + `@versioned` Field

When both store-level `@versioned` and field-level `@versioned` are present, you get **two layers of history**:

1. **Record-level** (from store `@versioned`): whole-record snapshots on every mutation
2. **Field-level** (from field `@versioned`): per-field granular history with diff support

```jade
store Document @versioned
    title as str
    body as text @versioned
    author as str

doc is documents.insert(title is 'Draft', body is 'First draft', author is 'Alice')
doc.body is 'Second draft'          # record version 2 + body field version 2
doc.title is 'Final'                 # record version 3 (body field unchanged at v2)
doc.body is 'Final content'          # record version 4 + body field version 3

# Record-level history (whole snapshots)
doc.history()                        # [v1, v2, v3, v4] — full records

# Field-level history (just the body field)
doc.body.history()                   # [v1: 'First draft', v2: 'Second draft', v3: 'Final content']

# Field-level history at a specific record version
old_doc is doc.at_version(2)
old_doc.body.history()               # [v1: 'First draft', v2: 'Second draft'] — only versions that existed at record v2
```

On disk:
```
data/documents/
    data/
        documents.store          # current (head) row data
        documents.versions       # append-only record version log
        body.versions            # append-only field version log
    idx/
        ...
```

The record version log stores whole-record snapshots. The field version log stores per-field values. Both are indexed by `(sid, version_num)`. The field version log entries are tagged with the record version they belong to, enabling queries like "field history at record version N."

---

## Schema Evolution and Migrations

### Compile-Time Schema Detection

The compiler knows the store schema from `store` declarations and the on-disk schema from the `.schema` file. When they diverge, the compiler classifies the changes:

1. **Field added**: trivial migration — zero-fill new field in existing records. `@default(value)` if specified.
2. **Field removed**: warning — data kept on disk (lazy cleanup), ignored in queries.
3. **Field type changed**: breaking — requires explicit migration.
4. **Field renamed**: breaking — requires explicit migration with `rename` directive.
5. **Relation added**: trivial — `.rel` file created, empty for existing records.
6. **Relation removed**: warning — `.rel` file retained, orphan records flagged.

For trivial changes (field added with default, field removed), the compiler auto-generates the migration code. For breaking changes, the programmer must write an explicit migration.

### Migration System — Environment-Aware, Runtime-Executed

**Migrations are NOT applied at compile time.** They are compiled INTO the application binary and executed at runtime with environment awareness. This is analogous to Flyway, Liquibase, or Rails ActiveRecord migrations — but native, compiled, and zero-dependency.

#### Why Runtime, Not Compile-Time
- The compiler doesn't know which environment it's building for (local, dev, stage, prod)
- Different environments may be at different schema versions
- Migrations may need to run on remote data stores or shared databases
- Stage/prod deployments may need to apply a batch of migrations atomically
- Rollback behavior differs per environment

#### Migration Declaration

```jade
migration 'add_verified_field' version 2
    up
        alter users
            add verified as bool default false

    down
        alter users
            drop verified

migration 'rename_username' version 3
    up
        alter users
            rename username to name

    down
        alter users
            rename name to username

migration 'add_profile_relation' version 4
    up
        alter users
            add &profile as Profile

    down
        alter users
            drop &profile

migration 'change_age_type' version 5
    up
        alter users
            transform age from i32 to i64 with *val
                val as i64

    down
        alter users
            transform age from i64 to i32 with *val
                val as i32
```

Migrations are ordered by version number. Each migration has an `up` (forward) and `down` (rollback) block. The `alter` block supports: `add`, `drop`, `rename`, `transform` (type change with conversion function).

#### Migration Tracking

Each store directory contains a `migrations.log` file — a ledger of applied migrations:

```
data/users/
    users.schema
    users.migrations.log       # [version, timestamp, direction, checksum]
    data/
        users.store
        ...
```

The `migrations.log` records which migrations have been applied:
```
[8B version][8B timestamp][1B direction (up=1, down=0)][32B checksum]
```

On startup, the runtime reads the migration log and compares it to the compiled migrations in the binary. Unapplied migrations are queued for execution.

#### Environment-Aware Execution

```jade
# In project.jade or via environment variable
store_migration is 'auto'         # auto-apply on startup (local/dev)
store_migration is 'prompt'       # prompt before applying (stage)
store_migration is 'manual'       # never auto-apply, require explicit command (prod)
store_migration is 'off'          # skip migration checks entirely
```

```jade
# Programmatic migration control
migrations is users.pending_migrations()
log 'pending: {migrations.length} migrations'

for m in migrations
    log 'v{m.version}: {m.name}'

# Apply all pending
users.migrate()

# Apply up to a specific version
users.migrate(to is 5)

# Rollback
users.rollback(to is 3)       # runs `down` for versions 5, 4

# Dry run — print what would happen without modifying data
users.migrate(dry_run is true)
```

#### Stacking Across Environments

Migrations stack. In local/dev, they may be applied one-at-a-time as developers iterate. When deploying to stage or prod, the runtime detects all unapplied migrations and applies them as a batch:

```
Local:   [v1] → [v2] → [v3] → [v4] → [v5]   (applied incrementally)
Dev:     [v1] → [v2] → [v3] → [v4] → [v5]   (applied incrementally)
Stage:   [v1] → [v2, v3, v4, v5]              (batch on deploy)
Prod:    [v1] → [v2, v3, v4, v5]              (batch on deploy, after stage validates)
```

Batch application wraps all migrations in a single transaction — if v4 fails, v2 and v3 are rolled back too. The migration log records the batch application with a single timestamp.

#### Data Migrations

For complex migrations that need to transform data (not just schema):

```jade
migration 'normalize_emails' version 6
    up
        for user in users.all()
            user.email is user.email.lowercase()

    down
        # data migrations may be irreversible
        error 'cannot reverse email normalization'
```

Data migration blocks have access to the store API — they can read and write records. The migration runs inside a transaction with exclusive write access.

#### Compiled Into Binary

The compiler collects all `migration` blocks at compile time and embeds them in the binary as functions. At runtime, the migration engine:

1. Reads the migration log from each store
2. Compares applied versions to compiled versions
3. Based on `store_migration` policy: auto-apply, prompt, or wait for manual trigger
4. Applies pending migrations in order, inside a transaction
5. Updates the migration log

No separate migration tool. No SQL scripts. No runtime interpreter. The migration code is native machine code compiled from Jade, running at the same speed as the rest of the program.

---

## Concurrent Access

### Single Process (default)
- Reader-writer lock per store: multiple concurrent readers, exclusive writer
- Lock-free reads for `@mem` stores (epoch-based snapshot isolation)
- Write coalescing: batch multiple writes into a single disk I/O
- Relation traversals acquire read locks on the target store automatically

### Multi-Process
- File-level `flock` for cross-process safety
- WAL enables concurrent readers with a single writer
- Future: shared-memory region for cross-process cache sharing

### Actor Integration
Stores integrate naturally with Jade's actor system:

```jade
actor UserService
    @create_user first_name as str, last_name as str, email as str, age as i64
        u is users.insert(first_name is first_name, last_name is last_name, email is email, age is age)
        reply u   # JadePack-serialized — zero-copy if local actor

    @find_user id as i64
        reply users.get(id)
```

Each actor can own a store handle. The actor mailbox serializes writes. Multiple actors can read concurrently. JadePack is the serialization format for both the mailbox and the store, so passing a store record to an actor is a direct `memcpy` — no re-serialization.

---

## Reactive Queries

```jade
# Watch for changes — returns a channel
changes is users.watch(age gt 21)

for event in changes
    match event
        Insert row => log 'new user: {row.first_name}'
        Update old, new => log 'updated: {old.first_name} -> {new.first_name}'
        Delete row => log 'removed: {row.first_name}'
```

Implemented via trigger hooks on the store's write path. Insert/update/delete operations check registered watchers and push events to channels.

### Relation Triggers

```jade
# Watch when related data changes
order_events is customers.watch_related(orders)

for event in order_events
    match event
        Insert customer, order => log '{customer.name} placed order #{order.sid}'
```

Combined with actors, this enables event-driven architectures without external message brokers.

---

## Views

Views are **read-only, computed projections** over one or more stores. They do not hold data — they define a query that materializes on access. Views compile to native code, just like store queries.

### Basic View

```jade
view ActiveUsers from users
    where deleted is 0
    where age gte 18

# Use like a store (read-only)
actives is active_users.all()
count is active_users.count()
page is active_users.paginate(50)
young is active_users.select(age lt 30)
```

A view is a named query. Every access to `active_users` re-executes the underlying query. There is no cached state — the view always reflects the current store data.

### View with Projection

```jade
view UserSummary from users
    pluck first_name, last_name, email, age
    where deleted is 0
    sort last_name

# Returns lightweight records with only the projected fields
summaries is user_summaries.all()
for s in summaries
    log '{s.first_name} {s.last_name}'
```

Projected views only read the fields they need. For column-store-backed fields, this means reading only the relevant column files — no wasted I/O.

### View with Joins

```jade
view OrderDetail from orders
    join customers on customer
    join order_items on items
    pluck customer.name, total, status, items
    where status neq 'cancelled'
    sort -created

details is order_details.select(total gt 100.0)
for d in details
    log '{d.customer.name}: ${d.total} ({d.items.length} items)'
```

Views can traverse relationships. The compiler resolves the joins at compile time using the same relation infrastructure as `&` fields.

### Materialized View

```jade
view TopSellers from orders @materialized
    group items.product_name
    sum total as revenue
    count as order_count
    sort -revenue
    limit 100
```

A `@materialized` view computes its results once and caches them. The cache is invalidated when the underlying store(s) are mutated. Re-materialization happens:
- **Lazily**: on next access after invalidation (default)
- **Eagerly**: immediately on underlying mutation (`@materialized(eager)`)
- **Periodically**: on a schedule (`@materialized(interval is 3600)` — refresh every hour)

```jade
# Force refresh
top_sellers.refresh()

# Check staleness
if top_sellers.stale()
    top_sellers.refresh()
```

Materialized views persist their cache to disk (a `.view` file in the store directory). On startup, the cached data is loaded — no re-computation unless the underlying data has changed.

### View Composition

Views can be built on top of other views:

```jade
view ActiveOrders from orders
    where status in ['pending', 'processing']

view HighValueActiveOrders from active_orders
    where total gt 1000.0
    sort -total
```

### View Constraints

- Views are **read-only**. You cannot insert, update, or delete through a view.
- Views do not have built-in fields (sid, uuid, etc.) — they inherit from the underlying store.
- Views compile to the same native code as the equivalent method call or query block. Zero overhead.
- `@materialized` views trade freshness for speed — suitable for dashboards, reports, aggregation.

---

## Vectorized Execution

For analytical queries — aggregations, group-by, windowed computations — Jade processes data in **batches of contiguous typed values** (vectors), not row-by-row. This enables SIMD instructions, cache-prefriendly access patterns, and branch-free computation. Inspired by DuckDB's columnar-vectorized model and Apache Arrow's format.

### When Vectorized Execution Activates

The compiler decides between row-at-a-time and vectorized execution at compile time:

| Operation | Execution Model |
|-----------|----------------|
| `.get(sid)`, `.first(...)` | Row — single record lookup |
| `.select(...)` with filter | Row — compiled predicate per record |
| `.sum()`, `.avg()`, `.min()`, `.max()` | Vectorized — SIMD over column data |
| `.group(...).sum(...)` | Vectorized — hash aggregate over column vectors |
| `.count()` | Direct — read count from metadata (O(1)) |
| Query block with aggregation | Vectorized |
| Time-series `.window(...)` | Vectorized — SIMD over chunk data |

### Vector Processing Model

```
Store data (row or column)
        │
        ▼
┌──────────────────┐
│  Scan Operator   │ → reads N records into a batch (default N=1024)
└──────────────────┘
        │
        ▼
┌──────────────────┐
│ Filter Operator  │ → applies compiled predicate, produces selection vector
└──────────────────┘
        │
        ▼
┌──────────────────┐
│  Agg Operator    │ → SIMD sum/avg/min/max over selected values
└──────────────────┘
        │
        ▼
       Result
```

Each operator processes a **vector** of values (a contiguous array of one type) rather than individual rows. For a `sum(age)` over 1M users:
- Row-at-a-time: 1M iterations, 1M branch predictions, scattered memory access
- Vectorized: ~1000 iterations over 1024-element batches, SIMD `vpaddd` instructions, linear memory scan

### Column Data for Vectorized Queries

When a store has column files (automatic for `@mem` stores, explicit for disk stores), aggregation reads column data directly:

```
age.col: [30, 25, 42, 18, 55, 31, 28, ...]  ← contiguous i64 array
```

The aggregation operator loads 1024 values at a time into a SIMD register and processes them with a single instruction. No record deserialization, no field extraction, no type dispatch.

For stores without column files, the vectorized engine extracts the relevant field from each record in the batch — still vectorized, but with the overhead of field extraction from fixed-size records.

### SIMD Primitives

The compiler generates SIMD code for common aggregation patterns:

| Operation | x86_64 | aarch64 |
|-----------|--------|---------|
| `sum(i64)` | `vpaddq` (AVX2/AVX-512) | `addv` (NEON) |
| `sum(f64)` | `vaddpd` | `faddp` |
| `min(i64)` | `vpminq` | `sminv` |
| `max(f64)` | `vmaxpd` | `fmaxnmv` |
| `count(predicate)` | `vpcmpq` + `popcnt` | `cmeq` + `addv` |

Fallback: scalar loop for architectures without SIMD or for non-standard types. The compiler selects the best instruction set available at compile time (with runtime feature detection as a future optimization).

---

## Specialized Store Types

### Vector Store (`@vector(N)`)

For semantic search, embeddings, similarity queries. Each record contains an N-dimensional float vector plus optional metadata fields.

```jade
store Embedding @vector(768)
    label as str @index
    source as str
    category as str @index

# Insert with vector data — the vector is the first positional argument
embeddings.insert([0.1, 0.2, ...768 floats...], label is 'doc_42', source is 'corpus_a')

# Or from a variable
vec is compute_embedding(text)
embeddings.insert(vec, label is 'doc_42', source is 'corpus_a')

# Nearest-neighbor search — returns records sorted by distance
similar is embeddings.nearest(query_vec, 10)               # top 10 nearest
similar is embeddings.nearest(query_vec, 10, category is 'science')  # with filter

# Distance metrics
similar is embeddings.nearest(query_vec, 10, metric is 'cosine')      # default
similar is embeddings.nearest(query_vec, 10, metric is 'euclidean')
similar is embeddings.nearest(query_vec, 10, metric is 'dot')

# Access distance from query
for result in similar
    log '{result.label}: distance {result.distance}'

# Threshold-based search — all vectors within distance
close is embeddings.within(query_vec, 0.5)                  # cosine distance < 0.5
close is embeddings.within(query_vec, 0.5, metric is 'euclidean')

# Query block syntax
similar is query embeddings
    nearest query_vec
    where category is 'science'
    limit 10
```

Implementation:
- Vector data stored in a separate `.vec` file — contiguous `N × f32` arrays for SIMD-friendly access
- Index: HNSW (Hierarchical Navigable Small World) graph for approximate nearest-neighbor (ANN)
  - Build time: O(n log n), query time: O(log n) for k-NN
  - Index file: `<store>.hnsw` in `idx/`
- For small stores (< 10K vectors): exact brute-force scan with SIMD dot product
- Metadata fields indexed normally (`@index`, `@sorted`) — combined with similarity search via pre-filter or post-filter
- Vectors are NOT included in JadePack encoding of the record — they live in the `.vec` file and are referenced by offset. This keeps the row store flat and the vector data contiguous for SIMD
- `@mem` vector stores load the HNSW index into memory — sub-millisecond k-NN on millions of vectors

### Graph Store (`@graph`)

For relationship-heavy data where traversal patterns are the primary access mode. Records are edges in a directed graph.

```jade
store Knows @graph
    &from as Person
    &to as Person
    since as i64
    strength as f64

store Person
    name as str @index
    age as i64
```

Graph stores have `&from` and `&to` as required relationship fields. They define edges between nodes (which are records in other stores).

```jade
# Insert edges
knows.insert(from is alice, to is bob, since is 2020, strength is 0.9)
knows.insert(from is bob, to is carol, since is 2021, strength is 0.7)

# Traverse — outgoing edges from a node
alice_knows is knows.from(alice)
for edge in alice_knows
    log '{edge.to.name} since {edge.since}'

# Incoming edges to a node
who_knows_bob is knows.to(bob)

# Pathfinding — shortest path between two nodes
path is knows.path(alice, carol)
for edge in path
    log '{edge.from.name} -> {edge.to.name}'

# Depth-limited traversal — all nodes within N hops
network is knows.traverse(alice, depth is 3)

# Filtered traversal — only strong connections
close is knows.traverse(alice, depth is 2, strength gt 0.8)

# Subgraph — extract a connected component
component is knows.subgraph(alice)

# Aggregation on graph — degree counts
popular is knows.degree_in()       # returns records sorted by incoming edge count
connectors is knows.degree_out()   # sorted by outgoing edge count

# PageRank (iterative)
ranks is knows.pagerank(iterations is 20, damping is 0.85)
for r in ranks
    log '{r.node.name}: {r.score}'

# Query block syntax
friends is query knows
    from alice
    where strength gt 0.5
    sort -strength
    limit 10
```

Implementation:
- Edge list stored as row data (standard store format)
- Adjacency index: two files per graph store — `<store>.adj_out` (from → edges) and `<store>.adj_in` (to → edges)
- Adjacency indexes are compressed sparse row (CSR) format — O(1) lookup of all edges from/to a node
- Traversal uses BFS/DFS with visited-set tracking — compiled to native loops, no interpreter
- Pathfinding: bidirectional BFS for shortest path, A* when edge weights are present
- PageRank: compiled to a tight SIMD loop over the adjacency matrix when `@mem`
- All graph operations compose with standard store filters — `knows.from(alice, since gt 2020)` uses index + filter

### Time-Series Store (`@timeseries(field)`)

For temporal data with a designated timestamp field. Optimized for time-range queries, windowed aggregation, and retention policies.

```jade
store SensorReading @timeseries(timestamp)
    timestamp as i64
    device_id as str @index
    temperature as f64
    humidity as f64
    pressure as f64
```

The `@timeseries(field)` decorator designates which field is the time axis. This field must be `i64` (unix timestamp) and is implicitly `@sorted`.

```jade
# Insert — timestamp can be explicit or auto-generated
readings.insert(timestamp is now(), device_id is 'sensor_01', temperature is 22.5, humidity is 45.0, pressure is 1013.25)

# Time range query
recent is readings.range(now() - 3600, now())                            # last hour
recent is readings.range(now() - 3600, now(), device_id is 'sensor_01')  # filtered

# Windowed aggregation — bucket by time interval
hourly is readings.window(3600).avg(temperature)                           # 1-hour buckets
hourly is readings.window(3600, device_id is 'sensor_01').avg(temperature)
daily is readings.window(86400).max(temperature)

# Multiple aggregations per window
stats is readings.window(3600)
    .avg(temperature)
    .max(humidity)
    .min(pressure)
    .count()

# Downsampling — reduce resolution for long-term storage
readings.downsample(source_window is 60, target_window is 3600, method is 'avg')

# Retention policy — auto-delete records older than duration
readings.retention(days is 90)         # records older than 90 days are auto-purged
readings.retention(count is 1000000)   # keep at most 1M records

# Latest value per group (last-value cache)
latest is readings.latest(group is device_id)
for r in latest
    log '{r.device_id}: {r.temperature}°C'

# Rate of change
rate is readings.rate(temperature, device_id is 'sensor_01', window is 3600)

# Query block syntax
hot_days is query readings
    where device_id is 'sensor_01'
    where timestamp gt (now() - 86400 * 7)
    where temperature gt 30.0
    sort -timestamp
    limit 100
```

Implementation:
- Data stored in time-partitioned chunks — each chunk covers a configurable time range (default: 1 hour)
- Chunk files: `<store>.<chunk_id>.ts` in `data/` — ordered by timestamp within each chunk
- Time index: B-tree on the timestamp field (implicit `@sorted`) — range queries are O(log n) seek + sequential scan
- Partition pruning: time-range queries skip chunks entirely when outside the range
- Windowed aggregation:
  - Column-oriented within each chunk for SIMD-friendly access
  - Pre-computed rollups for common intervals (minute, hour, day) when `@mem`
  - Incremental rollup: new inserts update the current window's aggregate without re-scanning
- Retention: background coroutine checks chunk timestamps, deletes expired chunks — no per-record overhead
- Downsampling: reads source chunks, computes aggregates, writes to new chunks at lower resolution, deletes originals
- Compression: delta encoding on timestamps (high value), RLE on repeated device IDs, standard float compression on values
- `@mem`: current chunk (and recent chunks up to memory budget) kept in memory for sub-microsecond latest-value queries

### Key-Value Store (`@kv`)

A persistent, in-memory-first key-value store. Replaces Redis, Consul, memcached, etcd — no external daemon, no TCP overhead, no serialization boundary. Keys are strings, values are any JadePack-serializable type. Compiled to native code like every other store.

```jade
store Cache @kv

store Session @kv @mem          # force all data in memory (default is memory-first anyway)
store Ephemeral @kv @transient  # pure in-memory, no persistence
store Config @kv @versioned     # versioned KV — values retain history on change
```

`@kv` stores have no field declarations. The store itself is the map.

```jade
# Basic get/set
cache.set('user:42', alice)                # value is JadePack-serialized
user is cache.get('user:42') as User       # type-annotated retrieval

# With TTL (time-to-live) — auto-expires after duration
cache.set('session:abc', token, ttl is 3600)     # expires in 1 hour
cache.set('rate:ip:1.2.3.4', count, ttl is 60)   # expires in 60 seconds

# Check existence and delete
if cache.has('user:42')
    cache.del('user:42')

# Get with default — returns default if key missing or expired
count is cache.get('counter', default is 0) as i64

# Atomic increment/decrement (value must be numeric)
cache.incr('counter')               # +1
cache.incr('counter', 5)             # +5
cache.decr('counter')                # -1

# Atomic compare-and-swap
success is cache.cas('lock:resource', expected is 'free', value is 'held')

# Bulk operations
cache.mset({'a': 1, 'b': 2, 'c': 3})
values is cache.mget(['a', 'b', 'c'])       # returns map

# Pattern matching on keys — glob-style
user_keys is cache.keys('user:*')
session_keys is cache.keys('session:*')

# Delete by pattern
cache.del('session:*')               # delete all sessions

# Iterate all key-value pairs
for key, value in cache.entries()
    log '{key}: {value}'

# Paginated iteration for large KV stores
pages is cache.paginate(100)
while pages.has_next()
    batch is pages.next()            # Vec of (key, value) pairs
```

#### Lists, Sets, and Sorted Sets

Like Redis, `@kv` stores support typed collection values:

```jade
# Lists — ordered, duplicates allowed
cache.lpush('queue:jobs', job)           # push left
cache.rpush('queue:jobs', job)           # push right
job is cache.lpop('queue:jobs')          # pop left
job is cache.rpop('queue:jobs')          # pop right
len is cache.llen('queue:jobs')
items is cache.lrange('queue:jobs', 0, 10)   # range slice

# Sets — unordered, unique
cache.sadd('tags:42', 'rust')
cache.sadd('tags:42', 'compiler')
members is cache.smembers('tags:42')     # {'rust', 'compiler'}
is_member is cache.sismember('tags:42', 'rust')  # true
cache.srem('tags:42', 'rust')            # remove member

# Set operations
union is cache.sunion('tags:42', 'tags:43')
inter is cache.sinter('tags:42', 'tags:43')
diff is cache.sdiff('tags:42', 'tags:43')

# Sorted sets — scored members, ordered by score
cache.zadd('leaderboard', 'alice', 1500.0)
cache.zadd('leaderboard', 'bob', 1200.0)
top is cache.zrange('leaderboard', 0, 10)            # top 10 by score
top is cache.zrange('leaderboard', 0, 10, rev is true) # bottom 10
score is cache.zscore('leaderboard', 'alice')         # 1500.0
rank is cache.zrank('leaderboard', 'alice')            # 0 (highest)
```

#### Pub/Sub

`@kv` stores support publish/subscribe channels — integrated with Jade's channel system:

```jade
# Subscribe — returns a Jade channel
ch is cache.subscribe('events:user')

# In an actor or coroutine
for msg in ch
    log 'received: {msg}'

# Publish
cache.publish('events:user', 'user:42:updated')

# Pattern subscribe — glob on channel names
ch is cache.psubscribe('events:*')
```

#### Expiry and Eviction

```jade
# Set TTL on an existing key
cache.expire('user:42', 300)           # expire in 300 seconds
cache.persist('user:42')               # remove TTL, keep forever

# Check remaining TTL
remaining is cache.ttl('user:42')      # seconds until expiry, -1 if no TTL

# Eviction policy (store-level configuration)
store HotCache @kv @mem
    @maxmemory(256)                    # max 256MB memory usage
    @eviction('lru')                   # evict least-recently-used when full
    # policies: 'lru', 'lfu', 'random', 'ttl' (evict nearest-expiry first)
```

Implementation:
- **Memory-first**: all data lives in a hash map in memory. Reads are O(1) pointer lookups — no syscalls, no I/O
- **Persistent backend**: WAL + periodic snapshot to disk (same as `@mem` stores). Crash-safe. On restart, replay WAL then load snapshot
- **TTL**: lazy expiry (checked on access) + active expiry (background coroutine sweeps expired keys every second)
- **Pub/sub**: zero-copy delivery to local subscribers via Jade channels. No TCP, no serialization boundary
- **Collections** (list/set/sorted set): stored as JadePack-encoded values under the key. Small collections inline in the value; large collections spill to an overflow segment
- **Atomic ops**: `incr`, `decr`, `cas` are lock-free CAS loops on the in-memory hash map
- **No external daemon**: the KV store is compiled into your program. No Redis server to manage, no connection pooling, no network latency. A `@kv` store in Jade is a native data structure with a disk backing
- **Composable**: `@kv @versioned` gives a versioned KV store (undo/redo, config audit trails). `@kv @transient` gives pure in-memory (session caches). `@kv @mem` is redundant but valid (KV is memory-first by default)
- **JadePack values**: values are JadePack-serialized. Any type the compiler knows how to pack can be a value — structs, enums, vecs, nested types. Type annotation on `.get()` tells the compiler which `unpack` function to call

---

## Hooks

Pre/post hooks on store operations for validation, side effects, and derived data:

```jade
store User
    first_name as str
    last_name as str
    email as str @index @unique
    email_domain as str @transient

    @before_insert self
        if self.email.length equals 0
            error 'email required'

    @after_insert self
        log 'user created: {self.email}'

    @before_save self
        self.email_domain is self.email.split('@').last

    @after_delete self
        log 'user deleted: {self.email}'
```

Hooks run inside the transaction. `@before_insert` can reject the operation by raising an error. `@before_save` fires before any write (insert or update). Hooks are compiled to inline native code — no dynamic dispatch.

---

## Compression (Column Store)

For column store files:

| Encoding | When Used | Benefit |
|----------|-----------|---------|
| Run-Length (RLE) | Low-cardinality columns (status, category) | 10-100x compression |
| Delta | Sorted or sequential numeric columns (timestamps, IDs) | 5-20x compression |
| Dictionary | String columns with < 65K distinct values | 3-10x + faster comparison |
| Bit-packing | Small integers, booleans | 2-8x compression |

Encoding selection is automatic based on column statistics collected during bulk-load or first N inserts.

---

## File Layout

For a standard store named `users`:

```
data/
  users/
    users.schema              # field manifest, schema hash
    users.metadata            # statistics, record count, index stats
    users.migrations.log      # applied migration ledger
    data/
        users.store           # row-oriented data (JadePack records)
        users.wal             # write-ahead log
        users.text            # heap file for Text fields (bio)
        users.json            # heap file for JSON fields (if any)
    rels/
        address.rel           # has-one join table (user_sid → address_sid)
        messages.rel          # has-many join table (user_sid → [message_sids])
    cols/
        age.col               # column file (if column store enabled)
    idx/
        email.idx             # hash index (@index)
        age.btree             # B-tree index (@sorted)
```

For a `@versioned` store named `documents` (with `body as text @versioned`):

```
data/
  documents/
    documents.schema
    documents.metadata
    documents.migrations.log   # applied migration ledger
    data/
        documents.store        # current (head) row data
        documents.versions     # append-only record version log (store-level @versioned)
        body.versions          # append-only field version log (field-level @versioned)
        documents.text         # text heap for overflow text fields
    rels/
    cols/
    idx/
        versions.btree         # B-tree on (sid, version_num) for record version lookups
```

For a `@vector(768)` store named `embeddings`:

```
data/
  embeddings/
    embeddings.schema
    embeddings.metadata
    data/
        embeddings.store      # metadata fields (label, source, etc.)
        embeddings.vec        # contiguous N×f32 vector data
    idx/
        embeddings.hnsw       # HNSW approximate nearest-neighbor index
        label.idx             # standard hash index on metadata
```

For a `@graph` store named `knows`:

```
data/
  knows/
    knows.schema
    knows.metadata
    data/
        knows.store           # edge records (from_sid, to_sid, fields...)
    idx/
        knows.adj_out         # from → [edge offsets] (CSR format)
        knows.adj_in          # to → [edge offsets] (CSR format)
```

For a `@timeseries(timestamp)` store named `readings`:

```
data/
  readings/
    readings.schema
    readings.metadata
    data/
        readings.1720000.ts   # time-partitioned chunk (chunk_id = start timestamp / interval)
        readings.1723600.ts   # next hourly chunk
        ...
    idx/
        timestamp.btree       # B-tree on timestamp (implicit @sorted)
        device_id.idx         # standard hash index
    rollups/
        minute.rollup         # pre-computed 1-minute aggregates
        hour.rollup           # pre-computed 1-hour aggregates
```

For a `@kv` store named `cache`:

```
data/
  cache/
    cache.schema              # KV store metadata (eviction policy, maxmemory)
    cache.metadata            # key count, memory usage stats
    data/
        cache.kv              # snapshot file — serialized hash map (JadePack)
        cache.wal             # write-ahead log for crash recovery
        cache.overflow        # overflow segment for large collection values
    idx/
        cache.ttl             # TTL index — sorted by expiry timestamp for active sweep
```

All store directories live under the path specified by `store_path` in `project.jade`.

---

## Full Example

```jade
store_path is 'data'
store_persistence is 'always'

store Address
    street as str
    city as str
    state as str @index
    zip as str @index

store Tag @simple
    label as str @index @unique

store User @mem
    first_name as str
    last_name as str @index
    email as str @index @unique
    age as i64 @sorted
    bio as text
    &address as Address @cascade
    &tags as [Tag] @cascade
    rank as i64 @increment

    *full_name returns str
        '{first_name} {last_name}'

store Order
    total as f64
    status as str @index
    notes as text @versioned
    &customer as User @required
    &items as [OrderItem] @cascade

store OrderItem @simple
    product_name as str
    quantity as i64
    price as f64

store Embedding @vector(384)
    label as str @index
    source as str

store Knows @graph
    &from as User
    &to as User
    since as i64
    strength as f64

store Metric @timeseries(timestamp)
    timestamp as i64
    endpoint as str @index
    latency_ms as f64
    status_code as i64

store AuditLog @versioned
    action as str @index
    entity as str
    detail as text @versioned
    &actor as User

store Cache @kv

store Session @kv @mem
    @maxmemory(64)
    @eviction('lru')

*main
    # --- Standard store operations ---

    # Insert
    addr is addresses.insert(street is '123 Main St', city is 'Portland', state is 'OR', zip is '97201')
    alice is users.insert(first_name is 'Alice', last_name is 'Smith', email is 'alice@test.com', age is 30, address is addr)

    # Access relation
    log alice.full_name              # 'Alice Smith'
    log alice.address.city           # 'Portland'

    # Insert related records
    order is orders.insert(total is 49.99, status is 'pending', customer is alice)
    order.items.add(order_items.insert(product_name is 'Widget', quantity is 2, price is 24.99))
    order.items.add(order_items.insert(product_name is 'Gadget', quantity is 1, price is 0.01))

    # Direct field mutation (auto-persisted)
    alice.age is 31

    # Batch update with .set()
    alice.set(age is 32, last_name is 'Jones')

    # Query — method API
    smiths is users.select(last_name is 'Smith')
    alice is users.first(email is 'alice@test.com')
    alice is users.get(1)            # by sid

    # Complex filter with OR
    results is users.select(
        or(
            and(last_name is 'Smith', age gt 30),
            and(last_name is 'Jones', age gt 25)
        )
    )

    # Query — query block syntax
    local is query users
        where address.state is 'OR'
        sort last_name
        limit 50

    # Paginator
    pages is users.paginate(25, age gt 18, sort is 'last_name')
    while pages.has_next()
        batch is pages.next()
        for u in batch
            log u.full_name

    # Aggregation
    avg_age is users.avg(age)
    log 'average age: {avg_age}'
    by_status is orders.group(status).sum(total)

    # Versioned field
    order.notes is 'Customer called — expedite shipping'
    order.notes is 'Shipped via priority mail'
    history is order.notes.history()

    # Versioned store (record-level versioning)
    log_entry is audit_logs.insert(action is 'create', entity is 'user', detail is 'Created Alice', actor is alice)
    log_entry.detail is 'Created Alice (updated)'     # creates record version 2
    log 'audit log versions: {log_entry.version_count()}'   # 2
    log_entry.rollback(1)                                     # reverts to v1, creates v3

    # Soft delete / restore
    users.delete(last_name is 'Smith')
    alice.restore()

    # Transaction
    transaction
        bob is users.insert(first_name is 'Bob', last_name is 'Jones', email is 'bob@test.com', age is 25, address is addr)
        orders.insert(total is 99.99, status is 'pending', customer is bob)

    # --- Vector store ---

    vec is compute_embedding('machine learning paper')
    embeddings.insert(vec, label is 'paper_42', source is 'arxiv')

    query_vec is compute_embedding('neural networks')
    similar is embeddings.nearest(query_vec, 5, metric is 'cosine')
    for s in similar
        log '{s.label}: {s.distance}'

    # --- Graph store ---

    bob is users.first(first_name is 'Bob')
    knows.insert(from is alice, to is bob, since is 2020, strength is 0.9)

    path is knows.path(alice, bob)
    network is knows.traverse(alice, depth is 2)

    # --- Time-series store ---

    metrics.insert(timestamp is now(), endpoint is '/api/users', latency_ms is 12.5, status_code is 200)

    hourly_latency is metrics.window(3600, endpoint is '/api/users').avg(latency_ms)
    latest is metrics.latest(group is endpoint)

    # --- Key-value store ---

    cache.set('user:42', alice)
    cache.set('session:xyz', token, ttl is 3600)
    cached_user is cache.get('user:42') as User

    cache.incr('page_views')
    cache.lpush('queue:emails', email_job)
    cache.zadd('leaderboard', alice.full_name, alice.rank)

    top_10 is cache.zrange('leaderboard', 0, 10)
    for entry in top_10
        log '{entry.member}: {entry.score}'

    # Session store with TTL
    sessions.set('sess:abc123', session_data, ttl is 1800)

    # --- Export / import ---

    users.export('users_backup.jpack')
    bytes is pack(alice)
    alice2 is unpack User bytes

    # --- Reactive ---

    changes is users.watch()
    # ... consume in an actor or coroutine

    log 'total users: {users.count()}'
```

---

## Design Critique and Trade-Offs

### Strengths
- **Whole-record I/O.** Records are contiguous fixed-size blobs. Load = one `memcpy`. Save = one `memcpy`. Collections = bulk range copy. Dynamic fields (text, json) use indirection pointers resolved in a second pass. No field-by-field serialization, no ORM hydration overhead.
- **Method API eliminates keyword pollution.** `users.insert()`, `users.select()`, `users.delete()` are regular method calls. No `insert`/`select`/`delete` reserved words. Composable with pipelines, variables, actor messages.
- **Dual API (methods + query blocks).** Methods for programmatic use and pipelines; query blocks for readable multi-line queries. Same compiled output. No forced choice.
- **Transparent persistence.** `alice.age is 31` looks like a normal assignment. The compiler intercepts it because `alice` is a store record. No explicit save calls needed in `'always'` mode. Feels like working with in-memory data.
- **Compile-time everything.** Schema validation, index selection, join table generation, pack/unpack codegen, FK resolution, type checking of filters — all happen at compile time. Zero runtime overhead for structural decisions.
- **Vectorized analytics.** Aggregations and analytical queries process data in SIMD-friendly batches over contiguous column data. Inspired by DuckDB's columnar-vectorized execution model. Row store for OLTP, column store for OLAP — same data, chosen automatically.
- **Views.** Named read-only projections over stores, including materialized views with auto-refresh. Views compile to native code, compose with the full query API, and enable dashboard/report patterns without denormalization.
- **Environment-aware migrations.** Schema migrations compile into the binary and execute at runtime with environment awareness. Stackable across environments (local→dev→stage→prod). Batch application, transactional rollback, dry-run support. No external migration tool.
- **JadePack unification.** One wire format for disk, mailbox, network. No impedance mismatch between "how data is stored" and "how data is sent." Content-addressable hashing falls out for free.
- **Relations via join tables.** Decoupling FKs from the record itself means records are flat and fixed-size. Join tables are small, indexed, and cacheable. Adding/removing relations doesn't require record migration.
- **Specialized stores as first-class citizens.** Vector, graph, time-series, and key-value are not bolt-on libraries — they share the same JadePack format, indexing infrastructure, decorator system, and query compilation pipeline. A graph edge IS a store record. A KV store IS a native hash map with a disk backing.
- **KV store replaces Redis/Consul/memcached.** No external daemon, no TCP, no connection pool. O(1) in-memory reads compiled to native code. TTL, pub/sub, lists, sets, sorted sets — all the Redis primitives, but in-process with zero serialization boundary.
- **Paginator with snapshot isolation.** Large result sets don't blow memory. Cursor-based traversal is safe under concurrent writes. Paginators are serializable via JadePack.
- **Record-level versioning.** `@versioned` stores retain previous record versions on mutation — editing history per record, not per store. Enables rollback, audit trails, diff between versions. Compaction controls disk growth.

### Trade-Offs
- **Join tables add indirection.** Has-one lookups require a `.rel` file read instead of an inline FK. Mitigated by `@mem` (in-memory graph) and automatic indexing. For non-`@mem` stores, the `.rel` file is mmap'd and effectively free for hot data.
- **256B string limit.** Strings that exceed 248 usable bytes must be `text`, which has overflow page overhead. This is a deliberate split — fixed-size fields enable O(1) record access and zero-copy reads. Most real-world strings (names, emails, codes) fit in 248 bytes.
- **Eager relations by default.** Loading all related records up front can be wasteful for has-many with many children. `@lazy` mitigates this, but the programmer must opt in. Default-eager prevents N+1 query problems at the cost of potentially loading unused data.
- **`@versioned` storage cost.** Store-level `@versioned` stores a full record copy in the version log on every mutation. Versioned fields grow linearly with mutations. For frequently-updated records, version chains can consume significant disk. Compaction via `.compact(keep is N)` or `.compact(older_than is N)` is the mitigation. Automatic compaction with retention policies is a future optimization.
- **No partial updates on disk.** Updating one field rewrites the full record (JadePack is fixed-layout). For large records with many string fields, this means writing ~N×256 bytes even for a single i64 change. `.set()` mitigates by batching, but the full record is still rewritten. Column store mitigates for analytics; row store prioritizes simplicity and crash safety.
- **HNSW index build cost.** Vector store insertions must update the HNSW graph — O(log n) per insert. Bulk loading should use batch construction. The index is approximate — exact k-NN requires brute-force fallback.
- **Graph traversal unbounded.** Deep traversals without depth limits can touch the entire graph. The `depth` parameter is recommended but not enforced. Future: compile-time warning for unbounded traversals.
- **Time-series chunk management.** Partition pruning requires sorted inserts. Out-of-order inserts force chunk splitting or merge. The `@timeseries` decorator assumes roughly monotonic timestamps — wildly out-of-order data degrades performance.
- **OR clauses disable index optimization.** `or(a, b)` may require scanning both index paths and merging. The query planner handles this, but worst-case is two index lookups + merge vs one lookup for pure AND.
- **KV type erasure.** Values in `@kv` stores are opaque JadePack blobs. The compiler cannot type-check `.get()` return values without the `as Type` annotation. A wrong annotation is a runtime unpack failure, not a compile error. Mitigation: typed KV wrappers (`store TypedCache @kv(User)`) are a future extension.

### Why Not Inline FKs?
Inline FKs (storing `address_id` directly in the User record) are simpler and faster for has-one. But join tables give us:
- **Bidirectional traversal** without schema changes to the child store
- **Many-to-many** with the same mechanism as one-to-many
- **Relation add/remove** without record rewrite or schema migration
- **Consistent model** — one concept (`&`) for all cardinalities

The indirection cost is negligible for `@mem` stores (in-memory graph) and amortized by mmap for disk stores.

---

## Implementation Priority

### Phase 1 — Foundation
1. **Record memory model** — contiguous fixed-size struct layout, whole-record `memcpy` load/save
2. **Method API** — `.insert()`, `.get()`, `.select()`, `.first()`, `.delete()`, `.destroy()`, `.save()` on store objects
3. **Built-in fields** (sid, uuid, hash, created, updated, deleted)
4. **Store decorators** (@simple, @mem, @transient) in parser/AST/HIR
5. **Field decorators** (@index, @unique, @sorted, @transient, @increment, @required, @versioned, @default)
6. **`store_path` / `store_persistence`** project configuration
7. **Direct field mutation** — `record.field is value` intercepted for store records
8. **`.set()` batch updates** — single WAL entry, single timestamp, single hash

### Phase 2 — Persistence + Query Power
9. **WAL + crash recovery**
10. **Soft delete / destroy / restore** semantics
11. **`store_persistence is 'explicit'`** — dirty tracking, `.save()` flush
12. **Complex filters** — `or()`, `and()`, `not()`, `in`, `between`, `like`
13. **Query blocks** — lower `query ... where ... sort ... limit` to same IR as method API
14. **Paginator / cursor** — `.paginate(N)`, `.next()`, `.has_next()`, snapshot isolation
15. **Dynamic fields** — `text` and `json` types with indirection pointers, heap files, transparent access

### Phase 3 — Relationships
16. **`&field as Type`** (has-one) — join table generation, auto-join
17. **`&field as [Type]`** (has-many) — join table, `.add()` / `.remove()`
18. **Relationship decorators** (@cascade, @lazy, @required)
19. **Relation filters** — `users.select(address.state is 'OR')` and `query ... where address.state is 'OR'`

### Phase 4 — JadePack
20. **Wire format spec** — encoding rules, header format
21. **Compiler-generated pack/unpack** — LLVM IR emission for every store type, whole-record `memcpy` for fixed-size records
22. **Actor message integration** — replace ad-hoc mailbox packing with JadePack
23. **`.export()` / `.import()`** methods, `.jpack` file format
24. **`pack()` / `unpack()`** built-in functions
25. **Paginator cursor serialization** via JadePack

### Phase 5 — Indexing + Memory
26. **Hash index** (`@index`)
27. **B-tree index** (`@sorted`)
28. **Uniqueness enforcement** (`@unique`)
29. **`@mem`** — in-memory store with background flush, bulk range `memcpy` for collections
30. **mmap** for non-`@mem` stores

### Phase 6 — Advanced Queries + Aggregation
31. **Aggregation** — `.sum()`, `.avg()`, `.min()`, `.max()`, `.count()`
32. **Grouping** — `.group(field).sum(field)`
33. **Projection** — `.pluck(field, field, ...)`
34. **`.distinct()`**, `.exists()`
35. **Field-level versioning** — `@versioned`, `.history()`, `.at_version()`, `.diff()`

### Phase 7 — Record-Level Versioning
36. **Store `@versioned`** — record version chains, `.history()`, `.at_version()`, `.rollback()`, `.diff()` per record
37. **Combined store + field versioning** — two-layer history (record snapshots + field-level diffs)
38. **Compaction** — `.compact(keep is N)`, `.compact(older_than is N)`, per-record compaction

### Phase 8 — Schema Migrations
39. **Compile-time schema detection** — auto-detect field added/removed, classify trivial vs breaking
40. **Migration declaration** — `migration` blocks with `up`/`down`, `alter` with `add`/`drop`/`rename`/`transform`
41. **Migration tracking** — `migrations.log` ledger per store, version comparison on startup
42. **Environment-aware execution** — `store_migration` config (`auto`/`prompt`/`manual`/`off`), programmatic `.migrate()`, `.rollback()`, `.pending_migrations()`
43. **Batch application** — transactional batch apply for stage/prod deployments, dry-run support
44. **Data migrations** — migration blocks with store API access for data transformation

### Phase 9 — Views
45. **Basic views** — `view Name from Store` with `where`, `sort`, `pluck` clauses, read-only
46. **Views with joins** — relation traversal in view definitions
47. **Materialized views** — `@materialized`, cached results, invalidation, lazy/eager/periodic refresh
48. **View composition** — views built on other views

### Phase 10 — Specialized Stores
49. **Vector store** (`@vector(N)`) — `.vec` file, HNSW index, `.nearest()`, `.within()`, SIMD dot product
50. **Graph store** (`@graph`) — adjacency indexes (CSR), `.from()`, `.to()`, `.path()`, `.traverse()`, `.pagerank()`
51. **Time-series store** (`@timeseries(field)`) — chunked partitions, `.range()`, `.window()`, `.retention()`, `.downsample()`, rollups
52. **Key-value store** (`@kv`) — in-memory hash map, WAL + snapshot persistence, `.set()`, `.get()`, `.del()`, TTL, `incr`/`decr`, `cas`
53. **KV collections** — lists (`lpush`/`rpush`/`lpop`/`rpop`), sets (`sadd`/`srem`/`sunion`/`sinter`), sorted sets (`zadd`/`zrange`/`zrank`)
54. **KV pub/sub** — `.publish()`, `.subscribe()`, `.psubscribe()`, integration with Jade channels
55. **KV eviction** — `@maxmemory`, `@eviction('lru'|'lfu'|'random'|'ttl')`, active + lazy expiry

### Phase 11 — Vectorized Execution + Column Store
56. **Column store** — per-field `.col` files, dual row+column representation for `@mem`
57. **Vectorized aggregation** — batch processing of column vectors, SIMD codegen for sum/avg/min/max
58. **SIMD primitives** — AVX2/AVX-512 on x86_64, NEON on aarch64, scalar fallback
59. **Query planner** — index vs scan, join strategy selection, OR-clause index merging, vectorized vs row-at-a-time decision

### Phase 12 — Polish + Optimization
60. **Store methods** — computed properties on store types
61. **Hooks** — `@before_insert`, `@after_insert`, `@before_save`, `@after_delete`
62. **Reactive queries** — `.watch()`, `.watch_related()`, channel integration
63. **Bloom filters** — auto-generated for frequently queried non-indexed fields
64. **Concurrent access** — reader-writer locks, epoch-based snapshot isolation, actor store handles
65. **Compression** — RLE, delta, dictionary, bit-packing for column files and time-series chunks
66. **Full-text search** — `@search`, inverted index
67. **Remote actor wire format** — JadePack over TCP
68. **Cross-process shared memory cache**
