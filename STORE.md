# Store — Compile-Time Adaptive Persistence Engine

**Version:** 0.3 (Full Specification)
**Status:** Design
**Lineage:** v0.1 flat-file → v0.2 adaptive layouts → v0.3 query engine + distributed + compression

---

## 0. Heritage

### 0.1 What v0.1 Proved

1,456 lines of inline LLVM IR. Flat binary (`JADESTR\0` magic, 24-byte header, fixed-size records). Linear scans. `flock` advisory locking. 256-byte fixed string buffers. Compound AND/OR filters. 17 tests pass.

The benchmark inserts 10k rows and queries 1k — works, but O(n) per query, strings truncated to 248 bytes, no indexes, no projection, delete rewrites the whole file, endianness baked in. Enough to prove the concept. Not enough to ship.

### 0.2 What v0.2 Designed

Layout annotations (`@row`, `@columnar`, `@hybrid`). B-tree and hash indexes. Field projection. Aggregations. Views. Schema aliases. `move`/`merge`. LRU/LFU/full cache modes. Self-describing v2 format with string heap. SSO for short strings. Cross-platform LE encoding.

v0.2 answered "what features." v0.3 answers "how a query actually executes, how data actually compresses, how stores actually distribute."

### 0.3 Lessons Absorbed

| System | Key Lesson Taken |
|--------|-----------------|
| **Apache Arrow** | Columnar memory format with fixed schemas enables zero-copy between systems. We adopt: fixed-width columns, dictionary encoding for strings, validity bitmaps for nullable fields. We diverge: no IPC flatbuffer overhead — our schema is compiled in, not serialized per-batch. |
| **DataFusion** | Logical → physical plan separation with rule-based + cost-based optimization. We adopt: the two-plan architecture, filter pushdown, projection pushdown, predicate reordering. We diverge: our "cost model" runs at compile time using type widths and annotation hints, not runtime statistics. |
| **DuckDB** | Vectorized pull-based execution on columnar chunks, morsel-driven parallelism, ART indexes, lightweight compression per-column. We adopt: chunk-based processing (not row-at-a-time), per-column compression selection, pushdown-everything. We diverge: we generate native LLVM IR instead of interpreting a bytecode — our "execution engine" is the compiled binary itself. |
| **Parquet** | Row groups + column chunks + page-level encoding = hierarchical storage divorced from hierarchical queries. We adopt: row-group segmentation for large stores, page-level compression, column statistics (min/max/null-count per chunk). We diverge: Parquet is read-optimized/immutable; our stores are mutable with in-place updates. |
| **SQLite** | B-tree pages, WAL mode, single-file database, compile-time optimization (`SQLITE_OMIT_*`), broad portability. We adopt: single-file-per-store simplicity, page-aligned I/O, WAL for crash recovery. We diverge: no SQL parser, no runtime query planner — it's all compiled. |
| **LevelDB / RocksDB** | LSM trees, write-optimized with background compaction, bloom filters for negative lookups. We adopt: bloom filters on indexed columns (skip scan when key definitely absent), tiered compaction for append-heavy stores. We diverge: no memtable/SSTable split — our row groups are self-contained. |

---

## 1. Design Axioms

1. **The compiler is the query planner.** Every `where`, `select`, `avg`, `order` compiles to a physical execution plan at build time. The plan is native LLVM IR. There is no interpreter, no VM, no bytecode.
2. **Zero dependencies.** The store engine is emitted as LLVM IR alongside user code. No SQLite. No Arrow runtime. No protobuf. The compiled binary is the database.
3. **Layout is per-schema.** Each store independently chooses row, columnar, or hybrid layout. The compiler respects annotations or infers from access patterns.
4. **Compression is per-column.** Each column independently chooses its encoding. The compiler selects based on type and annotation.
5. **Scale-invariant syntax.** `insert`, `where`, `count`, `all` — same whether 5 rows or 50 million. Performance differences come from layout, indexes, compression, and plan optimization — never from syntax.
6. **Format is self-describing and portable.** Explicit little-endian, schema embedded, no alignment assumptions. A `.store` written on x86_64 Linux opens on aarch64 macOS.
7. **Shards are stores. Stores are shards.** Multiple stores per schema creates an explicit shard topology. The compiler generates fan-out/gather for cross-shard queries.

---

## 2. Syntax — Complete Reference

### 2.1 Store Definition

```jade
store users
    name: String
    email: String
    age: i64
    active: bool
```

Fields with explicit types. Supported: `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32`, `f64`, `bool`, `String`.

### 2.2 Annotations

Annotations follow the store name or field name with `@`. Multiple annotations compose.

**Store-level:**

```jade
store events @columnar @compress @cache(4096)
    timestamp: i64 @index @sort
    kind: i64 @dict
    payload: String
    sensor_id: i64 @index(hash)
```

| Annotation | Level | Effect |
|------------|-------|--------|
| `@row` | store | Row-major layout (default) |
| `@columnar` | store | Column-major layout |
| `@hybrid(hot: f1, f2)` | store | Hot columns + cold row pack |
| `@compress` | store | Enable per-column compression (auto-select) |
| `@compress(lz4)` | store | Force LZ4 block compression |
| `@compress(none)` | store | Disable compression |
| `@cache(N)` | store | LRU cache of N records |
| `@cache(hot)` | store | Adaptive cache, auto-sized |
| `@cache(all)` | store | Full in-memory with write-back |
| `@mmap` | store | Memory-map data section |
| `@wal` | store | Write-ahead log for crash recovery |
| `@shard(field)` | store | Shard by field value (see §8) |
| `@remote(uri)` | store | Remote store endpoint (see §9) |
| `@page(N)` | store | Page size in bytes (default 4096) |

**Field-level:**

| Annotation | Effect |
|------------|--------|
| `@index` | B-tree index |
| `@unique` | B-tree index + uniqueness constraint |
| `@index(hash)` | Hash index (equality only) |
| `@sort` | Data physically sorted by this field (clustered) |
| `@dict` | Dictionary encoding (low-cardinality strings/ints) |
| `@delta` | Delta encoding (monotonic sequences) |
| `@rle` | Run-length encoding (runs of repeated values) |
| `@bloom` | Bloom filter for fast negative lookups |
| `@nullable` | Allow null values (validity bitmap) |

### 2.3 Core Operations

All v0.1 operations unchanged:

```jade
insert users 'Alice', 'alice@example.com', 30, true
u is users where name equals 'Alice'
n is count users
everyone is all users
delete users where age < 18
set users age 31 where name equals 'Alice'
transaction
    insert users 'Bob', 'bob@example.com', 25, true
    delete users where active equals false
```

### 2.4 Extended Operations

```jade
# Projection — returns lightweight struct with only requested fields
names is users select name, age where active equals true

# Aggregations — compile to single-pass accumulator loops
avg_age is users avg age
total  is users sum age where active equals true
oldest is users max age
youngest is users min age

# Count with filter
active_count is count users where active equals true

# Pagination
page is users where active equals true limit 20 offset 40

# Ordering
sorted is users where age > 18 order age desc limit 10

# Existence check — stops at first match, returns bool
has_alice is users exists where name equals 'Alice'

# Distinct
kinds is users distinct age

# Group-by aggregation (returns array of {key, agg} structs)
age_counts is users count group age

# Batch insert (array literal)
insert users batch [
    ('Alice', 'a@example.com', 30, true),
    ('Bob',   'b@example.com', 25, true),
    ('Carol', 'c@example.com', 35, false)
]

# Upsert — insert or update if @unique field conflicts
upsert users 'Alice', 'a-new@example.com', 31, true
```

### 2.5 Views

```jade
view active_users is users where active equals true
view young_active is users where age < 30 and active equals true

*main
    n is count active_users        # inlined as: count users where active equals true
    young is all young_active      # inlined with full filter
```

Views are compile-time macros. The compiler inlines the view's filter at each call site, then runs the full optimizer on the merged query. Zero runtime overhead.

### 2.6 Materialized Views

```jade
mview age_histogram is users count group age
    @refresh on insert, delete, set   # auto-refresh triggers
```

A materialized view persists its result to a companion store file. On listed triggers, the compiler generates refresh code. Between triggers, reads hit the materialized result — O(1) for pre-computed aggregations.

### 2.7 Schema Aliases & Multiple Stores

```jade
store archived_users as users       # same schema, separate file
store staging_users  as users       # another instance

# Move records between stores (transactional)
move users to archived_users where active equals false

# Merge all from source into target
merge staging_users into users

# Merge and clear source
merge staging_users into users and clear
```

### 2.8 Shard Groups

```jade
store logs @shard(year)
    year: i64
    timestamp: i64 @index
    level: i64 @dict
    message: String

# Query spans all shards transparently:
errors is logs where level > 3 order timestamp desc limit 100
```

The compiler generates one backing file per distinct shard-key value encountered (`logs_2024.store`, `logs_2025.store`, ...). Queries fan out across shard files, each sub-query runs in parallel (if `@parallel` enabled or auto-detected), results merge.

---

## 3. File Format — v3

### 3.1 Layout

```
 Offset  Section           Contents
 ──────  ────────────────  ──────────────────────────────────────
 0       File Header       128 bytes, fixed
 128     Schema            field descriptors, variable
 S       Column Metadata   per-column offset/size/encoding/stats
 M       Data Region       row groups × column chunks (or row blocks)
 D       String Heap       variable-length string payloads
 H       Index Region      B-tree / hash / bloom pages
 I       WAL Region        write-ahead log entries (optional)
 W       Footer            checksum, schema hash, EOF marker
```

### 3.2 File Header (128 bytes)

```
Offset  Size  Field             Description
──────  ────  ────────────────  ──────────────────────────────────
0       8     magic             "JADEST03" — format identifier
8       4     format_version    u32 LE — 3
12      4     flags             u32 LE — see flag bits below
16      8     record_count      u64 LE — total records
24      4     field_count       u32 LE — number of fields
28      4     row_group_count   u32 LE — number of row groups
32      8     schema_offset     u64 LE — byte offset to Schema section
40      8     colmeta_offset    u64 LE — byte offset to Column Metadata
48      8     data_offset       u64 LE — byte offset to Data Region
56      8     heap_offset       u64 LE — byte offset to String Heap
64      8     index_offset      u64 LE — byte offset to Index Region (0 = none)
72      8     wal_offset        u64 LE — byte offset to WAL Region (0 = none)
80      8     footer_offset     u64 LE — byte offset to Footer
88      4     page_size         u32 LE — page size in bytes (default 4096)
92      4     shard_id          u32 LE — shard identifier (0 for unsharded)
96      16    schema_hash       128-bit xxhash of schema (for alias validation)
112     16    reserved          zeros — future expansion
```

**Flag bits:**

| Bit | Mask | Meaning |
|-----|------|---------|
| 0–1 | 0x03 | Layout: 0=row, 1=columnar, 2=hybrid |
| 2 | 0x04 | Compression enabled |
| 3 | 0x08 | WAL enabled |
| 4 | 0x0C | Has indexes |
| 5 | 0x10 | Has bloom filters |
| 6 | 0x20 | Has dictionary pages |
| 7 | 0x40 | Sharded (shard_id is meaningful) |
| 8 | 0x80 | Uses mmap-compatible alignment |
| 9–31 | — | Reserved |

### 3.3 Schema Section

```
For each field (field_count entries):
  name_len:    u16 LE — length of field name in bytes
  name:        [name_len] UTF-8 bytes
  type_tag:    u8 — type enum (0=i8, 1=i16, ..., 10=f32, 11=f64, 12=bool, 13=String)
  flags:       u8 — field flags
  encoding:    u8 — compression encoding (0=none, 1=lz4, 2=dict, 3=rle, 4=delta, 5=zstd)
  index_type:  u8 — index (0=none, 1=btree, 2=hash, 3=btree+unique)
  padding:     u16 — reserved
```

Field flags: bit 0 = nullable, bit 1 = sorted, bit 2 = has bloom filter.

The complete schema is embedded so any tool can inspect a `.store` file without the Jade source.

### 3.4 Column Metadata

Per row-group, per column:

```
For each (row_group × field):
  col_offset:       u64 LE — byte offset of this column chunk in Data Region
  col_size:         u64 LE — compressed size in bytes
  col_raw_size:     u64 LE — uncompressed size
  num_values:       u64 LE — number of values in this chunk
  null_count:       u64 LE — number of nulls
  min_value:        [8B]   — min value (type-punned, for stats)
  max_value:        [8B]   — max value (type-punned, for stats)
  dict_page_offset: u64 LE — offset to dictionary page (0 if no dict)
  bloom_offset:     u64 LE — offset to bloom filter (0 if none)
```

**Why per-column stats matter:** The query planner uses min/max at compile time for static filter elimination (e.g., `where age > 200` on a column with max=98 → skip entire row group). At runtime for sharded/multi-row-group stores, min/max enables row-group pruning — same idea as Parquet RowGroup statistics.

### 3.5 Row Groups

Large stores split data into **row groups** (default 64K rows each). Each row group is independently readable — enables:
- Parallel scan (one thread per row group)
- Partial reads (skip row groups whose stats don't match the filter)
- Independent compression (better ratios on homogeneous chunks)

```
Row Group 0:  [col0 chunk][col1 chunk][col2 chunk]...
Row Group 1:  [col0 chunk][col1 chunk][col2 chunk]...
...
Row Group N:  [col0 chunk][col1 chunk][col2 chunk]...
```

For `@row` layout, each row group is a contiguous block of fixed-size records instead of column chunks.

### 3.6 String Heap

Variable-length strings with SSO:

```
String reference (in-record, 8 bytes):
  If len ≤ 7:
    [1B: tag=0x80|len][7B: inline data]          — inline SSO
  If len > 7:
    [1B: tag=0x00][3B: reserved][4B: heap_offset_hi]
    (actual offset is u48 stored across reserved+hi bytes)
    Heap entry: [4B: len_u32][len bytes: UTF-8 data]
```

The string heap lives in a dedicated section at the end of the file. For columnar layout, each column's string references point into the shared heap. The heap is append-only; deleted strings leave gaps reclaimed during compaction.

**Dictionary encoding** (`@dict`): Low-cardinality string columns store an array of unique values in a dictionary page, and each record stores a u32 dictionary index instead of a heap reference. Cuts storage for fields like `status`, `country`, `category` to 4 bytes/row regardless of string length.

### 3.7 Footer

```
checksum:    u64 LE — xxhash64 of entire file (excluding footer)
schema_hash: [16B] — duplicated from header (for quick validation)
magic:       [8B] — "JADEST03" (repeated, enables backward scanning)
```

---

## 4. Compression

### 4.1 Strategy: Per-Column, Compile-Time Selection

Each column selects its encoding independently. The compiler chooses based on type + annotation, or the programmer overrides.

| Encoding | Best For | How It Works |
|----------|----------|--------------|
| **None** | Small stores, latency-sensitive | Raw values. No overhead. |
| **LZ4** | General purpose, cold columns | Block compression per row-group chunk. Fast decompression (>4 GB/s). |
| **Zstd** | Archive stores, high compression ratio | Higher ratio than LZ4, slower. Good for `@compress(zstd)`. |
| **Dictionary** (`@dict`) | Low-cardinality strings & ints | Unique-value table + integer indices. 10× compression on `status` fields. |
| **Run-Length** (`@rle`) | Sorted columns with long runs | `(value, count)` pairs. Sorted `bool` column → 2 entries total. |
| **Delta** (`@delta`) | Monotonic sequences (timestamps, IDs) | Store first value + differences. Timestamps with 1ms resolution → differences fit in 2 bytes. |
| **Delta + Zigzag** | Near-monotonic sequences | Delta encoding where differences can go negative — zigzag-encode the deltas so small signed values are small unsigned values. |
| **Bit-packing** | Booleans, small enums | Pack N booleans into N/8 bytes. Pack 3-bit enum into ⅜ byte/value. |
| **Frame-of-reference** (FOR) | Integer columns with known range | Subtract minimum, store offset with fewer bits. Column of ages 18–65 → 6 bits/value. |

### 4.2 Auto-Selection Rules

When `@compress` is specified without an explicit algorithm:

```
if type == bool:                                  → bit-packing
if type == String && @dict annotation:            → dictionary
if type == String && estimated cardinality < 256: → dictionary (auto)
if type == String:                                → LZ4
if type ∈ {i8..u64} && @sort:                     → delta
if type ∈ {i8..u64} && @rle:                      → RLE
if type ∈ {i8..u64}:                              → FOR + LZ4
if type ∈ {f32, f64}:                             → LZ4
```

Cardinality estimation at compile time: the compiler analyzes insert statements visible in the source. If all inserts use literals, exact cardinality is known. Otherwise, annotations guide the decision.

### 4.3 Compression in the Query Pipeline

Compressed data affects the physical plan:

- **Dictionary-encoded columns:** Equality filter on `kind equals 'error'` → look up `'error'` in dictionary → compare u32 codes. No string comparison in the hot loop.
- **RLE-encoded columns:** Aggregation `sum` on an RLE column → multiply value × run-length. O(runs) instead of O(rows).
- **Delta-encoded columns:** Range filter `timestamp > T` → binary search on prefix sums. No full decode needed.
- **LZ4/Zstd columns:** Decompress one row-group chunk at a time into a scratch buffer. Process. Discard. Never decompress the entire column.

### 4.4 Codegen for Compression

Compression/decompression code is generated inline as LLVM IR, not linked from a library:

- **LZ4:** The decompressor is ~200 lines of LLVM IR. Single-pass, no allocations beyond the output buffer.
- **Dictionary:** A lookup table (global array) + u32 index dereference.
- **Delta:** Prefix-sum scan (SIMD-parallelizable — 4-wide prefix sum with SSE).
- **RLE:** Two pointers (value, run-length) walking in lockstep.
- **Bit-packing:** Shift + mask instructions. The compiler knows the bit-width at compile time.

This keeps Jade dependency-free. No `liblz4.so`, no `libzstd.a`. The entire compression stack compiles into the binary.

---

## 5. Query Engine — Logical and Physical Plans

### 5.1 Architecture

```
   Jade Source
       │
       ▼
   ┌────────┐
   │ Parser │   Syntactic AST: StoreQuery, StoreCount, Insert, Delete, ...
   └───┬────┘
       │
       ▼
   ┌────────┐
   │ Typer  │   Schema validation, type inference, struct synthesis
   └───┬────┘
       │
       ▼
   ┌────────────────┐
   │ Logical Planner │   Relational algebra: Scan → Filter → Project → Aggregate → Sort → Limit
   └───────┬────────┘
           │
           ▼
   ┌────────────────┐
   │  Optimizer      │   Rule-based rewriting, then cost-based plan selection
   └───────┬────────┘
           │
           ▼
   ┌────────────────┐
   │ Physical Planner│   Concrete operators: IndexSeek, SeqScan, VecScan, HashAgg, MergeSort, ...
   └───────┬────────┘
           │
           ▼
   ┌────────────┐
   │  Codegen    │   LLVM IR — the physical plan IS the generated code
   └────────────┘
```

Every query goes through this pipeline **at compile time**. The compiled binary contains only the physical plan as native machine code.

### 5.2 Logical Plan

The logical plan is a tree of relational operators, type-checked and schema-aware but layout-agnostic:

```rust
pub enum LogicalPlan {
    /// Full table scan — reads every record
    Scan {
        store: String,
        schema: Vec<(String, Type)>,
    },

    /// Filter — predicate pushdown target
    Filter {
        predicate: LogicalExpr,
        input: Box<LogicalPlan>,
    },

    /// Projection — column pruning
    Projection {
        fields: Vec<usize>,           // field indices to keep
        input: Box<LogicalPlan>,
    },

    /// Aggregation — sum, avg, min, max, count, group-by
    Aggregate {
        group_by: Vec<usize>,         // field indices for grouping
        aggregates: Vec<AggExpr>,     // (function, field_index)
        input: Box<LogicalPlan>,
    },

    /// Sort
    Sort {
        key: usize,
        direction: SortDir,
        input: Box<LogicalPlan>,
    },

    /// Limit + Offset
    Limit {
        limit: usize,
        offset: usize,
        input: Box<LogicalPlan>,
    },

    /// Distinct
    Distinct {
        fields: Vec<usize>,
        input: Box<LogicalPlan>,
    },

    /// Cross-store union (for shard fan-out)
    Union {
        inputs: Vec<LogicalPlan>,
    },

    /// Cross-store join (future)
    Join {
        left: Box<LogicalPlan>,
        right: Box<LogicalPlan>,
        on: (usize, usize),           // left_field, right_field
        kind: JoinKind,
    },
}

pub enum AggExpr {
    Count,
    Sum(usize),      // field index
    Avg(usize),
    Min(usize),
    Max(usize),
}

pub enum LogicalExpr {
    Column(usize),
    Literal(ScalarValue),
    BinOp(Box<LogicalExpr>, BinOp, Box<LogicalExpr>),
    And(Box<LogicalExpr>, Box<LogicalExpr>),
    Or(Box<LogicalExpr>, Box<LogicalExpr>),
    Not(Box<LogicalExpr>),
    IsNull(Box<LogicalExpr>),
}
```

### 5.3 Optimizer Rules

The optimizer applies **rewrite rules** to the logical plan. Each rule is a pattern-match transformation. Rules fire repeatedly until fixed-point (no rule matches).

| # | Rule | Effect | Inspired By |
|---|------|--------|-------------|
| 1 | **Filter Pushdown** | Push Filter below Projection, below Union (into each shard). | DataFusion `PushDownFilter` |
| 2 | **Projection Pushdown** | Push Projection below Filter — only load needed columns. | DataFusion `PushDownProjection` |
| 3 | **Predicate Reorder** | In compound AND predicates, cheapest-to-evaluate first (int compare < string compare < function call). | Cost-based. |
| 4 | **Constant Folding** | `where age > 10 + 5` → `where age > 15`. Evaluate compile-time-known expressions. | Standard. |
| 5 | **Dead Column Elimination** | If no downstream operator reads a column, exclude it from Scan/Projection. | DataFusion `ColumnPruning` |
| 6 | **Filter Fusion** | Adjacent Filters → single Filter with AND. | Standard. |
| 7 | **Limit Pushdown** | Push Limit below Sort when sort is on the same key as a clustered index (`@sort`). | DuckDB `TopN`. |
| 8 | **Aggregate Pushdown** | `count` with no filter → read record_count from header. `min`/`max` on `@sort` column → read first/last record. O(1). | Statistics shortcut. |
| 9 | **Row Group Pruning** | Use column statistics (min/max) to skip entire row groups whose range doesn't intersect the filter predicate. | Parquet/DataFusion. |
| 10 | **Bloom Filter Check** | If a field has `@bloom`, insert a bloom-filter check before the scan loop. If definitely-absent → skip row group entirely. | LevelDB/RocksDB. |
| 11 | **Shard Pruning** | For sharded stores, if the filter includes the shard key, eliminate shards that can't match. `logs where year equals 2025` → only open `logs_2025.store`. | Partition pruning. |
| 12 | **Dictionary Rewrite** | Filter on `@dict` column → look up the dictionary at compile time (if literal), convert filter to integer comparison. | Arrow DictionaryArray. |
| 13 | **Exists Short-Circuit** | `exists where ...` → Limit(1) on the scan. Stop after first match. | Standard. |
| 14 | **Union Collapse** | Union of a single input → remove Union wrapper. | Standard. |

### 5.4 Physical Plan

The physical plan maps each logical operator to a concrete implementation strategy:

```rust
pub enum PhysicalPlan {
    /// Sequential scan — row or column, compressed or raw
    SeqScan {
        store: String,
        layout: Layout,           // Row | Columnar | Hybrid
        columns: Vec<usize>,     // which columns to read
        row_groups: Vec<u32>,    // which row groups to scan (after pruning)
        decompress: Vec<Encoding>, // per-column decompression to apply
    },

    /// Vectorized column scan — SIMD-accelerated filter on columnar data
    VecScan {
        store: String,
        filter_col: usize,
        filter_op: BinOp,
        filter_val: ScalarValue,
        simd_width: u8,           // 4 (SSE2), 8 (AVX2), 16 (AVX-512)
        then: Option<Box<PhysicalPlan>>,
    },

    /// B-tree index seek
    IndexSeek {
        store: String,
        index_name: String,
        range: (Bound, Bound),
        then: Option<Box<PhysicalPlan>>,
    },

    /// Hash index lookup
    HashLookup {
        store: String,
        index_name: String,
        key: ScalarValue,
    },

    /// Filter (residual — after index seek, check remaining predicates)
    Filter {
        predicate: PhysicalExpr,
        input: Box<PhysicalPlan>,
    },

    /// Projection (reorder/subset columns in output)
    Project {
        fields: Vec<usize>,
        input: Box<PhysicalPlan>,
    },

    /// Streaming aggregation (input sorted on group-by key)
    StreamAggregate {
        group_by: Vec<usize>,
        accumulators: Vec<Accumulator>,
        input: Box<PhysicalPlan>,
    },

    /// Hash aggregation (unsorted input)
    HashAggregate {
        group_by: Vec<usize>,
        accumulators: Vec<Accumulator>,
        input: Box<PhysicalPlan>,
    },

    /// Top-N sort (heap-based, O(N log K) for limit K)
    TopN {
        key: usize,
        direction: SortDir,
        limit: usize,
        input: Box<PhysicalPlan>,
    },

    /// Full sort (fallback when no limit or limit > threshold)
    MergeSort {
        key: usize,
        direction: SortDir,
        input: Box<PhysicalPlan>,
    },

    /// Limit/Offset
    Limit {
        limit: usize,
        offset: usize,
        input: Box<PhysicalPlan>,
    },

    /// Parallel fan-out across shards
    ShardFanOut {
        shard_stores: Vec<String>,
        per_shard: Box<PhysicalPlan>,  // template plan, instantiated per shard
        merge: MergeStrategy,          // Concat | MergeSort | HashAggregate
    },

    /// Cache probe — check in-memory cache before falling through to scan
    CacheProbe {
        store: String,
        cache_mode: CacheMode,
        fallback: Box<PhysicalPlan>,
    },

    /// Bloom filter probe — pre-check before scan
    BloomProbe {
        store: String,
        field: usize,
        value: ScalarValue,
        on_absent: Box<PhysicalPlan>,   // empty result
        on_maybe: Box<PhysicalPlan>,    // fall through to real scan
    },

    /// Read result from materialized view file
    ReadMView {
        mview_store: String,
    },
}
```

### 5.5 Physical Plan Selection — The Compile-Time Cost Model

The optimizer scores candidate physical plans using a cost model that runs **entirely at compile time**:

| Factor | Source | Cost Weight |
|--------|--------|-------------|
| Column width | Schema (known at compile time) | Bytes to read per row |
| Layout overhead | `@row` vs `@columnar` annotation | Row: all columns read. Columnar: only needed columns. |
| Index availability | `@index` annotation | IndexSeek: O(log N) vs SeqScan: O(N) |
| Compression ratio (est.) | Encoding tag + type width | Compressed read < raw read |
| SIMD availability | `--target` flag / LLVM target features | VecScan vs scalar scan |
| Row group count | Estimated from record count if known | Parallelism granularity |
| Cache hit probability | `@cache` annotation + access pattern hints | CacheProbe overhead vs benefit |

The cost model **does not have runtime statistics** (no histogram, no sampled pages). It uses schema information + annotations as the sole input. This is a deliberate design choice:

- **Advantage:** Zero runtime overhead for plan selection. Plans are baked into the binary. No "first query is slow" warmup.
- **Tradeoff:** Suboptimal plans when data distribution is surprising. Mitigation: the programmer adds annotations (`@dict`, `@sort`, `@bloom`) to communicate distribution knowledge.

This is the opposite of PostgreSQL/DuckDB (runtime stats drive plan selection) and the same spirit as C++ template metaprogramming: push decisions to compile time, accept that the programmer must communicate what the compiler can't observe.

### 5.6 Example: Logical → Physical Compilation

```jade
store events @columnar @compress
    timestamp: i64 @index @sort @delta
    kind: i64 @dict
    payload: String
    sensor_id: i64 @index(hash) @bloom

# Query:
recent_errors is events where kind equals 3 and timestamp > 1700000000
    order timestamp desc limit 10
```

**Step 1 — Parse to Logical Plan:**
```
Limit(10, 0,
  Sort(timestamp, Desc,
    Filter(kind == 3 AND timestamp > 1700000000,
      Scan(events, [timestamp, kind, payload, sensor_id])
    )
  )
)
```

**Step 2 — Optimizer rewrites:**

Rule 2 (Projection Pushdown): query only uses `timestamp`, `kind`, `payload`, `sensor_id` — but since `select *` is implied, keep all. However, dead column elimination observes that downstream only binds `recent_errors` as a struct — all fields needed.

Rule 1 (Filter Pushdown): filters are already at the scan.

Rule 7 (Limit Pushdown): `timestamp` is `@sort` (clustered) + sort direction is `desc`. Limit can push below sort → TopN with reverse-order index walk.

Rule 12 (Dictionary Rewrite): `kind` is `@dict`. `kind equals 3` → resolved at compile time: look up dictionary page for value `3`, get dict index, rewrite to `kind_dict_idx == <idx>`.

Rule 9 (Row Group Pruning): `timestamp > 1700000000` → emit row-group-level min/max check at runtime. Skip row groups where `max_timestamp < 1700000000`.

**Step 3 — Physical Plan:**
```
TopN(key=timestamp, dir=Desc, limit=10,
  Filter(kind_dict_idx == <idx>,
    IndexSeek(events, "timestamp_btree",
      range=(1700000000, +∞),
      direction=Backward,     # reverse walk for desc order
      row_group_pruning=true,
      decompress=[delta, dict, lz4, none]
    )
  )
)
```

**Step 4 — Codegen:** emits a function that:
1. Opens the B-tree for `timestamp`, seeks to the largest key.
2. Walks backward (descending), decompressing delta-encoded timestamps.
3. For each candidate row, checks `kind_dict_idx` (integer compare, not string compare).
4. Collects up to 10 matches in a stack-allocated array.
5. Returns the array as a Jade struct array.

No linear scan. No sort step. No heap allocation beyond the 10-element result.

---

## 6. Storage Layouts — Deep Dive

### 6.1 Row Layout (`@row`)

Records stored contiguously, field-by-field within each record.

```
Record [i] = data_offset + i * record_stride

record_stride is computed at compile time:
  sum of (field_storage_size for each field) + alignment padding
```

**Strengths:** Fast single-record lookup by rowid. Fast insert (append). Good for write-heavy OLTP patterns. Simple codegen.

**Weaknesses:** Full-record I/O even when query touches 1 field. No SIMD opportunity. Poor compression (heterogeneous bytes within a record).

**When to use:** Small stores (< 10K records), stores with mostly full-record reads, stores where insert throughput matters more than query throughput.

### 6.2 Columnar Layout (`@columnar`)

Each field stored as a contiguous typed array. Each column is independently compressed and independently scannable.

Internally organized as column chunks within row groups:

```
Row Group 0:
  ┌────────────┬────────────┬────────────┐
  │ col0: i64  │ col1: dict │ col2: str  │  ← each independently compressed
  │ [64K vals] │ [64K vals] │ [64K refs] │
  └────────────┴────────────┴────────────┘

Row Group 1:
  ┌────────────┬────────────┬────────────┐
  │ col0: i64  │ col1: dict │ col2: str  │
  │ [64K vals] │ [64K vals] │ [64K refs] │
  └────────────┴────────────┴────────────┘
```

**Strengths:**

| Property | Why It Matters |
|----------|---------------|
| Cache-line efficiency | Scanning `age` column reads only `age` values, not `name`, not `email`. 8× less data for an 8-field store. |
| SIMD vectorization | `__m256i` loads 4 × i64 values at once. Compare, mask, branch-free filter. |
| Compression ratio | Same-type values cluster → delta, RLE, FOR all work dramatically better than on row data. |
| Projection is free | `select name, age` reads 2 column buffers. Other columns never touch memory. |
| Parallel scan | Row groups are independent. One thread per group, merge results. |

**Weaknesses:** Single-record lookup requires gathering values from N columns. Insert must append to N columns + possibly update N indexes. Not ideal for point-lookup-heavy workloads.

**When the compiler auto-selects columnar:** When the store has ≥ 4 fields AND the majority of queries are filtered scans or aggregations AND the store is not write-dominant.

### 6.3 Hybrid Layout (`@hybrid`)

Split fields into **hot set** (columnar) and **cold set** (row-packed):

```
Hot columns:           [timestamp][timestamp]...[value][value]...
Cold row blocks:       [{sensor_id, label}] [{sensor_id, label}]...
```

The hot set is determined by:
1. Explicit annotation: `@hybrid(hot: timestamp, value)`.
2. Query analysis: fields appearing in `where` / `order` / aggregation → hot.
3. Fallback heuristic: numeric fields → hot, string fields → cold (strings are larger and less SIMD-friendly).

Hot columns get all columnar benefits (SIMD, compression, projection). Cold fields are accessed only when the query actually needs them — which means after filtering on hot columns has already reduced the candidate set.

### 6.4 Adaptive Layout

When no annotation is given, the compiler chooses:

```
if field_count <= 3:                             @row    (too few fields for columnar wins)
if field_count >= 4 AND all_queries_are_scans:   @columnar
if field_count >= 4 AND mixed_access_patterns:   @hybrid (auto-partition)
if write_ratio > 0.7 (estimated):                @row    (write-optimized)
default:                                         @row
```

"All queries are scans" is determined by analyzing every `where`, `select`, `avg`, `sum`, `count` expression on the store in the current compilation unit. If the store is only inserted into and never queried, `@row` wins (simplest codegen, fastest append).

---

## 7. Indexes — Deep Dive

### 7.1 B-Tree (`@index`, `@unique`)

Page-aligned B+ tree. Leaf pages hold keys + row indices. Internal pages hold separator keys + child page offsets.

Design parameters (computed at compile time):

```
key_size      = type_width(field)                    // e.g., 8 for i64
ptr_size      = 8                                    // u64 page offset
page_size     = store's @page(N) or 4096
fan_out       = (page_size - 4) / (key_size + ptr_size)
leaf_capacity = (page_size - 4) / (key_size + 8)     // 8 = row_index size
```

For `i64` keys with 4096-byte pages: fan_out = 255, leaf_capacity = 255. A 3-level tree indexes 255³ ≈ 16.5 million rows. 4 levels → 4.2 billion rows.

**String keys:** Use prefix-truncated keys in internal nodes (first 32 bytes). Leaf nodes store full keys (heap references). Comparison falls through to the string heap only when prefixes match.

**Operations:**

| Operation | B-tree Effect |
|-----------|--------------|
| `insert` | Walk tree → find leaf → insert key. If full → split leaf, propagate. O(log N). |
| `delete` | Walk tree → find key → remove. If underflow → merge or redistribute. O(log N). |
| `set` | If update touches indexed field: delete old key, insert new key. If not: index unchanged. |
| `where field op val` | Seek to position → scan in range. O(log N + matches). |
| Bulk load (`batch insert`) | Sort keys → build leaves left-to-right → build internal nodes bottom-up. O(N log N). |

**Unique constraint (`@unique`):** On insert, check if key exists (B-tree point lookup). If yes → compile-time configurable behavior:
- Default: runtime error (program terminates with diagnostic).
- `@unique(ignore)`: silently skip the insert.
- `@unique(replace)`: overwrite the existing record (upsert semantics).

### 7.2 Hash Index (`@index(hash)`)

Linear-probing hash table in a dedicated section of the store file.

```
bucket_count: u64 (power of 2)
table: [bucket_count × entry]
  entry = { hash: u64, row_index: u64, occupied: u8 }
```

- Hash function: wyhash (fast, well-distributed, public domain).
- Load factor threshold: 0.6. On exceeding → double bucket_count, rehash.
- Tombstones on delete (lazy cleanup during rehash).
- O(1) amortized for equality lookups. No range support.
- If a query uses `>`, `<`, `>=`, `<=` on a hash-indexed field, the hash index is ignored and the planner falls back to scan or B-tree.

### 7.3 Bloom Filter (`@bloom`)

A space-efficient probabilistic structure for "is this key definitely NOT in the set?"

```
bits_per_key  = 10                  (1.2% false positive rate)
k_hash_funcs  = 7                   (optimal for 10 bits/key)
bitmap_size   = record_count * 10 / 8  bytes
```

Stored per row group. The query planner inserts a BloomProbe node before the scan: if the bloom filter says absent → skip this row group entirely. Reduces I/O for point lookups on non-indexed or secondarily-indexed columns.

### 7.4 Adaptive Radix Tree (ART) — Future

For string keys with common prefixes (URLs, file paths, identifiers), an ART index would provide:
- O(key_length) lookup instead of O(log N) B-tree comparisons.
- Better cache behavior for prefix-heavy workloads.
- Path compression for sparse key spaces.

Reserved for a future phase. The file format's index section is extensible (index_type tag in schema).

---

## 8. Sharding

### 8.1 Concept

A sharded store is a **set of store files** with the same schema, each holding a subset of records. The shard key determines which file a record belongs to.

```jade
store logs @shard(year)
    year: i64
    timestamp: i64 @index
    level: i64 @dict
    message: String
```

The compiler generates:
- `logs_2024.store`, `logs_2025.store`, `logs_2026.store`, ... — one per distinct shard key value.
- A **shard registry** file `logs.shards` — a JSON-like manifest listing all shard files and their key ranges.
- Insert: extract shard key → route to correct file (open lazily if needed).
- Query: logical plan's Scan is rewritten as `Union(Scan(shard_0), Scan(shard_1), ...)`.

### 8.2 Shard Key Types

| Pattern | Annotation | Behavior |
|---------|------------|----------|
| **Value-based** | `@shard(year)` | One file per unique value. Good for temporal partitions. |
| **Range-based** | `@shard(id, range=1000)` | Each file holds a range of 1000 IDs. `id 0–999` → shard 0, `1000–1999` → shard 1. |
| **Hash-based** | `@shard(user_id, hash=16)` | 16 shards, record → `hash(user_id) % 16`. Even distribution. |
| **Manual** | `store shard_X as logs` | Programmer controls shard topology directly. |

### 8.3 Shard Pruning

When the query includes the shard key in its filter:

```jade
errors is logs where year equals 2025 and level > 3
```

The optimizer's Rule 11 (Shard Pruning) eliminates all shards except `logs_2025.store`. Only one file is opened and scanned. This transforms a potentially multi-file fan-out into a single-store operation — same performance as an unsharded store.

### 8.4 Cross-Shard Queries

When the query does NOT include the shard key:

```jade
all_errors is logs where level > 3 order timestamp desc limit 100
```

Physical plan:
```
ShardFanOut(
    shards: [logs_2024, logs_2025, logs_2026],
    per_shard: TopN(key=timestamp, dir=Desc, limit=100,
        Filter(level_dict > 3, SeqScan(columns=[timestamp, level, message]))
    ),
    merge: MergeSort(key=timestamp, dir=Desc, limit=100)
)
```

Each shard returns its top 100. The merge step combines K sorted lists with a K-way merge (heap-based, O(N log K)). Total results = 100 even though each shard returned up to 100.

### 8.5 Shard Compaction & Rebalancing

Over time, value-based shards may become uneven (one year has 10M records, another has 100). Compaction can be triggered manually:

```jade
compact logs                          # merge small shards, split large ones
compact logs where year < 2020 into logs_archive   # cold-storage merge
```

The compiler generates a compaction function that reads source shard(s), rewrites into balanced target shard(s), and updates the shard registry atomically.

---

## 9. Remote & Distributed Stores

### 9.1 Remote Stores

```jade
store users @remote('tcp://db-primary:9400/users')
    name: String
    email: String @unique
    age: i64
```

A remote store delegates I/O to a network endpoint. The compiler generates:
- A **store client stub** instead of file I/O functions.
- A **wire protocol** (binary, length-prefixed messages) for insert/query/delete/set operations.
- Connection establishment on first access (lazy, like file opens).
- Retry logic with configurable timeout (default 5s).

The wire protocol is simple:

```
Message:
  [4B: msg_len][1B: op_code][payload...]

Op codes:
  0x01  INSERT       [schema_hash:16B][field_values...]
  0x02  QUERY        [schema_hash:16B][serialized_filter][limit:u64]
  0x03  DELETE       [schema_hash:16B][serialized_filter]
  0x04  SET          [schema_hash:16B][serialized_assignments][serialized_filter]
  0x05  COUNT        [schema_hash:16B]
  0x06  ALL          [schema_hash:16B][offset:u64][limit:u64]
  0x10  RESULT_ROWS  [count:u64][row_data...]
  0x11  RESULT_COUNT [count:u64]
  0x1F  ERROR        [code:u32][msg_len:u16][msg...]
```

Schema hash validation: the client sends its schema hash with every request. The server compares against its own schema hash and rejects mismatches. This catches accidentally connecting to a store with a different schema — a compile-time guarantee made robust at runtime.

### 9.2 Store Server

A Jade program can serve a store:

```jade
store users
    name: String
    email: String @unique
    age: i64

serve users on 9400    # binds TCP, accepts store protocol messages
```

The compiler generates a server loop:
1. Bind TCP socket on the specified port.
2. Accept connections (one thread per connection, or single-threaded with epoll/kqueue).
3. Parse incoming messages, dispatch to local store operations.
4. Return results via wire protocol.

The server is a compiled Jade binary — no separate database process. The "database" is the program.

### 9.3 Distributed Queries

Combine `@shard` with `@remote`:

```jade
store orders @shard(region, hash=4)
    region: String
    order_id: i64 @unique
    total: f64
    customer: String

# Shard endpoints:
store orders @remote('tcp://node-0:9400/orders') @shard_id(0)
store orders @remote('tcp://node-1:9400/orders') @shard_id(1)
store orders @remote('tcp://node-2:9400/orders') @shard_id(2)
store orders @remote('tcp://node-3:9400/orders') @shard_id(3)
```

A query fans out to all 4 remote shards in parallel (one connection per shard, async I/O), results merge locally:

```jade
big_orders is orders where total > 1000.0 order total desc limit 50
```

Physical plan:
```
ShardFanOut(
    shards: [remote_0, remote_1, remote_2, remote_3],
    per_shard: TopN(key=total, dir=Desc, limit=50, Filter(total > 1000.0, SeqScan)),
    merge: MergeSort(key=total, dir=Desc, limit=50),
    transport: TCP
)
```

The query pushdown is complete: each shard applies the filter and limit locally. Only 50 result rows travel over the network per shard. The local merge reduces 200 candidates to 50.

### 9.4 Consistency Model

Distributed stores use **eventual consistency** by default:

- Each shard is a single-writer, multi-reader store.
- Writes route to the shard that owns the record (determined by shard key hash/range).
- Reads can hit any shard that has the data (for replicated stores) or the owning shard.
- No distributed transactions in v0.3. Each shard's transaction is local.

For stronger guarantees:

```jade
store accounts @remote(...) @consistency(strong)
    ...
```

`@consistency(strong)` enables a two-phase commit protocol for cross-shard writes. Heavy — use only when needed.

---

## 10. Cache System

### 10.1 Architecture

The cache sits between the query engine and the storage layer:

```
Query → Cache Probe → [HIT] → Return cached result
                    → [MISS] → Storage Read → Populate Cache → Return
```

### 10.2 Implementation Modes

**`@cache(N)` — Fixed-size LRU:**

```
struct CacheEntry {
    row_index: u64,
    record: [u8; record_size],    // raw bytes
    prev: u32, next: u32,         // LRU doubly-linked list
}

struct Cache {
    entries: [CacheEntry; N],     // pre-allocated
    hash_table: [u32; N * 2],    // open-addressing, maps row_index → slot
    head: u32, tail: u32,         // LRU endpoints
    count: u32,
}
```

Generated as LLVM IR: a global struct, initialized on first access. The hash table uses linear probing. Eviction: on cache full, evict `tail` (least recently used).

**Query-result caching (for filtered queries):**

Cache key = hash of (filter predicate). Cache value = array of matching row indices.  
On mutation → invalidate ALL query-result cache entries (conservative but correct).

**`@cache(all)` — Full materialization:**

The entire store is loaded into a malloc'd buffer on first access. All queries run against the in-memory copy. Mutations apply to the in-memory copy AND flush to disk (write-back). Simplest mode. Only viable when the store fits in memory.

### 10.3 Invalidation

| Operation | Cache Effect |
|-----------|-------------|
| `insert` | New record added to cache (if cache not full) or replace LRU. Invalidate query-result cache. |
| `delete` | Remove matching entries from cache. Invalidate query-result cache. |
| `set` | Update matching entries in-place. Invalidate query-result cache. |
| Schema change (recompile) | Cache is fully invalidated (layout may have changed). |

---

## 11. Migration & Upgrade Strategy

### 11.1 Format Detection

The first 8 bytes identify the format:

| Magic | Version | Reader |
|-------|---------|--------|
| `JADESTR\0` | v0.1 | Legacy reader (fixed records, 256B strings) |
| `JADEST02` | v0.2 | Self-describing header, string heap |
| `JADEST03` | v0.3 | Row groups, column metadata, compression, footer |

On open, the runtime checks magic bytes and dispatches to the appropriate reader.

### 11.2 Automatic Migration

When a v0.3 compiled program opens a v0.1 or v0.2 file:

1. **Detect** — read first 8 bytes, identify version.
2. **Read** — use the legacy reader to load all records into memory.
3. **Convert** — write records to a new file in v0.3 format:
   - Embed schema from the Jade source's store definition.
   - Apply layout (`@row`/`@columnar`/`@hybrid` per annotation).
   - Build indexes if `@index` annotations are present.
   - Apply compression if `@compress` is annotated.
   - Populate column metadata (min/max/null_count per chunk).
   - Write footer with checksum.
4. **Backup** — rename original to `<name>.store.v1.bak` or `.v2.bak`.
5. **Replace** — rename new file to `<name>.store`.

Migration is atomic (rename is atomic on POSIX/NTFS). If the process crashes mid-migration, the original file is untouched.

### 11.3 Schema Evolution

When the Jade source changes a store's schema between compilations:

| Change | Handling |
|--------|----------|
| **Add field** (with default) | New field appended. Existing records get the default value. Migration rewrites all records. |
| **Add field** (without default) | Compile error: "field 'x' added without default — existing store data cannot be migrated." |
| **Remove field** | Migration rewrites records without the removed field. Original backed up. |
| **Rename field** | Not auto-detectable. Treated as remove + add. Use `@migrate(old_name → new_name)` annotation to guide. |
| **Change field type** | Compile error unless a coercion is defined: `@migrate(age: String → i64, *fn(s) parse_int(s))`. |
| **Change layout** | Full rewrite. Row → columnar rewrites all data into column chunks. |
| **Add index** | Build index from existing data during migration (bulk-load). |
| **Remove index** | Drop index pages. No data change. |

Schema hash (128-bit xxhash of field names + types + order) is stored in the header. On open, the compiled schema hash is compared to the file's schema hash. Mismatch → trigger migration.

### 11.4 Migration Annotations

```jade
store users @version(3)
    name: String
    email: String @unique
    age: i64
    active: bool @default(true)             # added in v3
    @migrate(2 → 3)
        add active default true             # migration instruction
```

The `@migrate(old → new)` block tells the compiler exactly how to transform records from version N to version N+1. Chain migrations apply sequentially: v1 → v2 → v3.

### 11.5 Offline Migration Tool

For large stores where in-process migration is too slow:

```bash
jadec migrate users.store --schema=main.jade --output=users_v3.store --parallel=4
```

The compiler emits a standalone migration binary that reads the old file, converts in parallel (one thread per row group), and writes the new file. Progress reporting to stderr.

---

## 12. Compile-Time Optimizations — The Jade Advantage

### 12.1 Why Compile-Time Matters

Traditional databases plan queries at runtime because they don't know the query until it arrives. Jade knows every query at compile time — they're written in the source code. This unlocks optimizations that are impossible in a runtime query engine:

| Optimization | Runtime DB | Jade |
|-------------|-----------|------|
| Query plan selection | Per-execution (parse + plan + execute) | Once at compile time. Zero per-query overhead. |
| Schema validation | Runtime check per query | Compile error. Invalid queries never reach a binary. |
| Index selection | Statistics-dependent, can choose wrong | Annotation-driven, deterministic. |
| Type dispatch | Dynamic dispatch per column type | Monomorphized. No vtable, no type tag checks. |
| String comparison | General-purpose function call | Inline LLVM IR specialized for the field's max observed length or encoding. |
| Compression codec | Runtime detection per page | Compiled in. Decompressor is native code with known encoding. |
| Vectorization | JIT (if available) | LLVM auto-vectorization + explicit SIMD intrinsics. |
| Dead column elimination | Runtime statistics | Static analysis. Columns never read are never loaded. |
| Constant folding | Basic | Full — LLVM's optimization passes fold everything the optimizer didn't catch. |
| Cross-query optimization | None (queries are independent) | The compiler sees all queries to a store and can co-optimize (shared indexes, layout choice). |

### 12.2 Cross-Query Analysis

The compiler analyzes **all operations on a store across the entire program**:

```jade
store products
    name: String
    price: f64
    category: String

*main
    # Query 1: filter on category
    cheap is products where category equals 'electronics' and price < 100.0

    # Query 2: aggregation on price
    avg_price is products avg price

    # Query 3: count
    n is count products
```

The compiler observes:
1. `category` appears in a filter → candidate for index.
2. `price` appears in a filter AND an aggregation → hot field.
3. `category` has literal equality check → candidate for `@dict` (if not already annotated).
4. `count` is called → store the count in the header (already done).

If the programmer didn't annotate, the compiler can **suggest** annotations:

```
note: store 'products' would benefit from @index on 'category' (used in filter at line 8)
note: store 'products' would benefit from @columnar (3 fields, 2 scan-based queries)
```

These are diagnostics, not auto-applied changes. The programmer decides. But the compiler has done the analysis.

### 12.3 Specialization Per Call Site

Different queries on the same store can compile to different physical plans:

```jade
# This compiles to an IndexSeek (B-tree on category)
electronics is products where category equals 'electronics'

# This compiles to a VecScan (SIMD scan on price column)
expensive is products where price > 1000.0

# This compiles to a header read (O(1))
n is count products
```

Three queries, three different code paths, all generated at compile time. A runtime engine would at best JIT-compile these; Jade AOT-compiles them.

### 12.4 Dead Store Elimination

If a store is declared but never queried (only inserted into), the compiler can:
- Skip generating query functions.
- Skip building indexes (since no query will use them — with a warning).
- Use the simplest possible insert path (append-only, no index maintenance).

Conversely, if a store is only queried (opened read-only at runtime), the compiler can:
- Skip generating insert/delete/set functions.
- Open the file read-only (fewer syscalls, can share `mmap` across threads).

---

## 13. Flex Storage — Runtime Layout Negotiation

### 13.1 Concept

For stores where the workload changes over time (e.g., batch-load phase followed by query phase), **flex storage** allows the runtime to switch layouts without recompilation:

```jade
store telemetry @flex
    timestamp: i64 @sort
    value: f64
    sensor: String @dict
```

`@flex` emits codegen for **both** row and columnar paths. At runtime:
- During bulk insert (batch of >1K rows): use append-only row layout for maximum write throughput.
- On first query after a batch: **compact** recent rows into columnar row groups. Background operation — subsequent queries hit columnar data.

This is similar to DuckDB's **row-group appends** + **background compaction** pattern.

### 13.2 Codegen Cost

`@flex` roughly doubles the generated code size for that store (row path + columnar path + compaction function). For most stores, the programmer knows the workload and should pick a fixed layout. `@flex` is the escape hatch for genuinely mixed workloads.

### 13.3 Hot-Cold Tiering

Combine `@flex` with time-based tiering:

```jade
store logs @flex @tier(hot: 7d, warm: 30d, cold: archive)
    timestamp: i64 @sort
    level: i64 @dict
    message: String
```

- **Hot (0–7 days):** Row layout, `@cache(all)`, no compression. Maximum write speed.
- **Warm (7–30 days):** Columnar row groups, `@compress(lz4)`, indexes active. Balanced.
- **Cold (30+ days):** Columnar, `@compress(zstd)`, dropped from cache. Maximum space efficiency.

The runtime emits background compaction that migrates rows from hot → warm → cold based on the `timestamp` field.

---

## 14. Concurrency & Transactions

### 14.1 Single-Process Concurrency

```jade
store accounts @wal
    id: i64 @unique
    balance: f64
    name: String

transaction
    a is accounts where id equals 1
    b is accounts where id equals 2
    set accounts balance (a.balance - 100.0) where id equals 1
    set accounts balance (b.balance + 100.0) where id equals 2
```

With `@wal`:
1. Write all mutations to the WAL (append-only log).
2. On commit → apply WAL entries to the main data file.
3. On crash → replay WAL on next open (redo logging).

WAL format:
```
[8B: txn_id][4B: op_code][variable: op_payload][4B: checksum]
```

Crash recovery: scan WAL forward, apply complete transactions (ones with a commit record), discard incomplete ones.

### 14.2 Multi-Process Access

`flock` (POSIX) / `LockFileEx` (Windows) for coarse-grained mutual exclusion. Same as v0.1 but now with WAL support:

- **Readers:** shared lock (multiple concurrent readers allowed).
- **Writers:** exclusive lock (single writer, blocks readers during write).

For better concurrency, `@wal` enables MVCC-like behavior:
- Writer appends to WAL without modifying the main file.
- Readers read from the main file (snapshot isolation — they don't see uncommitted writes).
- On commit, writer applies WAL to main file (briefly takes exclusive lock).

### 14.3 Isolation Levels

| Level | Annotation | Behavior |
|-------|-----------|----------|
| Read Uncommitted | (default without `@wal`) | Readers see partial writes. Fastest. |
| Read Committed | `@wal` | Readers see only committed data. |
| Snapshot | `@wal @snapshot` | Each transaction sees a consistent snapshot. Requires version tracking. |
| Serializable | `@wal @serializable` | Full serializability. Write-set conflict detection at commit. |

---

## 15. Per-Schema Configuration Summary

Every store independently selects its configuration. There is no global "storage engine mode." This is the key insight from DuckDB and the opposite of MySQL's storage-engine-per-server model.

```jade
# OLTP-style: row layout, B-tree indexes, WAL, cache
store accounts @row @wal @cache(1024)
    id: i64 @unique
    balance: f64
    name: String @index

# OLAP-style: columnar, compressed, no WAL (append-only analytics)
store events @columnar @compress @mmap
    timestamp: i64 @sort @delta
    kind: i64 @dict
    payload: String

# Hybrid: mixed workload, hot fields columnar, cold fields row-packed
store orders @hybrid(hot: total, status) @cache(hot) @wal
    order_id: i64 @unique
    customer_id: i64 @index
    total: f64
    status: String @dict
    notes: String

# Time-series: sharded by day, columnar, compressed, bloom filters
store metrics @columnar @compress @shard(day) @mmap
    day: i64
    timestamp: i64 @sort @delta
    sensor_id: i64 @bloom @index(hash)
    value: f64

# Remote: thin client stub, all I/O over TCP
store users @remote('tcp://db:9400/users')
    name: String
    email: String @unique
    age: i64

# Tiny reference table: cached entirely in memory
store config @row @cache(all)
    key: String @unique
    value: String
```

Each store compiles to its own set of LLVM IR functions, its own file(s), its own indexes, its own cache. No shared state between stores unless explicitly linked (`as`, `move`, `merge`).

---

## 16. Implementation Phases

### Phase A — Foundation

| # | Item | Changes | LOC Est. |
|---|------|---------|----------|
| A1 | Variable-length strings (SSO + heap) | stores.rs rewrite: string_heap section, str_ref encoding | ~400 |
| A2 | v3 file header + schema section | New header layout, schema serialization, magic upgrade | ~200 |
| A3 | Footer + checksum | xxhash64 of file body, append footer on close | ~80 |
| A4 | Projection (`select f1, f2`) | Parser + typer + codegen for subset struct construction | ~250 |
| A5 | Schema evolution detection | Hash-based mismatch → migration trigger | ~150 |
| A6 | Auto-migration v0.1 → v0.3 | Legacy reader, conversion, atomic rename | ~300 |

### Phase B — Query Engine

| # | Item | Changes |
|---|------|---------|
| B1 | Logical plan representation | New `LogicalPlan` enum in hir.rs |
| B2 | Logical → Physical planner | Cost model, plan selection |
| B3 | Optimizer rules (filter/projection pushdown, constant folding) | Rule engine with fixed-point iteration |
| B4 | B-tree index | On-disk B+ tree with page-aligned I/O, bulk-load |
| B5 | Hash index | Linear-probing hash table |
| B6 | Bloom filters | Per-column, per-row-group bit arrays |
| B7 | Aggregations (sum, avg, min, max) | Accumulator codegen |
| B8 | Order + Limit (TopN) | Heap-based top-N selection |
| B9 | Group-by (hash aggregate) | Hash table accumulation |

### Phase C — Columnar & Compression

| # | Item | Changes |
|---|------|---------|
| C1 | Columnar layout | Column-chunk writer/reader, row-group segmentation |
| C2 | Column metadata (min/max/null stats) | Per-chunk statistics, row-group pruning |
| C3 | Dictionary encoding (`@dict`) | Dictionary page + integer indices |
| C4 | Delta encoding (`@delta`) | Prefix-sum encode/decode |
| C5 | RLE (`@rle`) | Run-length encode/decode |
| C6 | LZ4 block compression | Inline LLVM IR LZ4 decompressor |
| C7 | Vectorized scan (SIMD) | AVX2/SSE2/NEON column filter codegen |
| C8 | Hybrid layout (`@hybrid`) | Hot/cold column partitioning |

### Phase D — Scale & Distribution

| # | Item | Changes |
|---|------|---------|
| D1 | Sharding (`@shard`) | Shard registry, insert routing, fan-out queries |
| D2 | Cache system (`@cache`) | LRU/LFU in-memory cache codegen |
| D3 | WAL (`@wal`) | Write-ahead log, crash recovery, replay |
| D4 | mmap backend (`@mmap`) | Memory-mapped data section |
| D5 | Views (`view`) | Compile-time query inlining |
| D6 | Materialized views (`mview`) | Persistent result store, refresh triggers |
| D7 | Schema migration annotations | `@migrate`, `@version`, migration codegen |
| D8 | Batch insert | Bulk-load path, deferred index build |
| D9 | Upsert | Unique-key check + conditional insert/update |

### Phase E — Remote & Distributed

| # | Item | Changes |
|---|------|---------|
| E1 | Wire protocol | Binary message format, serialize/deserialize |
| E2 | Remote store stub (`@remote`) | TCP client codegen |
| E3 | Store server (`serve`) | TCP server codegen, dispatch loop |
| E4 | Distributed shard fan-out | Parallel remote queries, merge sort |
| E5 | Replication | Leader/follower, async replication log |

### Phase F — Advanced

| # | Item |
|---|------|
| F1 | Flex storage (`@flex`) — runtime layout switching |
| F2 | Hot-cold tiering (`@tier`) |
| F3 | Cross-store joins |
| F4 | Nested struct / array fields |
| F5 | Zstd compression |
| F6 | Async I/O (io_uring / kqueue) |
| F7 | ART index for string keys |
| F8 | Change-data-capture hooks |
| F9 | Offline migration tool (standalone binary) |
| F10 | Adaptive layout auto-selection (cross-query analysis) |

---

## 17. Backward Compatibility

All v0.1 syntax remains valid. All v0.2 syntax remains valid. New features are additive.

```jade
# This v0.1 program compiles identically under v0.3:
store users
    name: String
    email: String
    age: i64

*main
    insert users 'Alice', 'alice@example.com', 30
    insert users 'Bob', 'bob@example.com', 25
    young is users where age < 30
    log count users
    log all users
```

The default configuration (no annotations) produces a `@row` store with no compression, no indexes, no cache, no WAL — functionally identical to v0.1 but with the v0.3 file format (self-describing, portable, extensible).

---

## 18. Design References

| Source | What We Took |
|--------|-------------|
| [Andy Grove, "How Query Engines Work"](https://leanpub.com/how-query-engines-work) | Logical/physical plan separation, rule-based optimization, accumulator-based aggregation, cost model structure |
| [Apache Arrow](https://arrow.apache.org/docs/format/Columnar.html) | Fixed-width columnar buffers, validity bitmaps, dictionary encoding, zero-copy philosophy |
| [Apache DataFusion](https://datafusion.apache.org/) | Filter pushdown, projection pushdown, predicate reordering rules, physical plan traits |
| [DuckDB](https://duckdb.org/internals/overview) | Vectorized execution model, morsel-driven parallelism, per-column compression, ART indexes |
| [Apache Parquet](https://parquet.apache.org/docs/) | Row groups, column chunks, page-level encoding, column statistics for pruning |
| [SQLite](https://sqlite.org/arch.html) | Single-file simplicity, B-tree page layout, WAL mode, compile-time configuration |
| [RocksDB](https://rocksdb.org/blog/) | Bloom filters for negative lookups, tiered compaction, write-optimized ingestion |
| [Perceus (Reinking et al.)](https://www.microsoft.com/en-us/research/publication/perceus-garbage-free-reference-counting-with-reuse/) | Compile-time memory management — directly analogous to compile-time query planning |

---

## Appendix A: v0.1 File Format (Legacy)

```
[8 bytes: magic "JADESTR\0"]
[8 bytes: record count (i64)]
[8 bytes: record size (i64)]
[N * record_size bytes: records...]

String field: [8B len][248B data]  — fixed 256B buffer
Bool field: 8 bytes (0 or 1 as i64)
```

Generated runtime functions (v0.1):

| Function | Purpose |
|----------|---------|
| `__store_<name>_ensure_open` | Lazy FILE* open |
| `__store_<name>_insert` | Seek-to-end, write record, update count |
| `__store_<name>_count` | Read count from header offset 8 |
| `__store_<name>_query` | Bulk-read all records, linear scan, first match |
| `__store_<name>_all` | Bulk-read, convert fixed-buf strings to Jade Strings |
| `__store_<name>_delete` | Read all, rewrite file without matches |
| `__store_<name>_set` | Read all, modify matches in buffer, write back |

Current implementation: 1,456 lines in [src/codegen/stores.rs](src/codegen/stores.rs). Locking via `flock`/`LOCK_EX` + `LOCK_UN`.
