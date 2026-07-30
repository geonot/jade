# Incremental compilation — requirements for a real design

The `src/incr.rs` artifact cache was deleted (task 8-20): its call sites
only ever logged a dirty count, `ArtifactCache::store` was never called
outside its own unit test (so `lookup` could never hit), and the design
could not have worked if wired up. Recorded here so the next attempt does
not rebuild the same broken shape.

Why the deleted design was unusable:

1. **Keys mixed in non-dependencies.** `function_cache_key` hashed every
   function's signature into every key, so any signature change anywhere
   dirtied everything. A usable key covers exactly the function's actual
   dependency set: the signatures it calls, the types it mentions, the
   constants it folds — discovered from the HIR, not globally.
2. **Span-sensitive hashing.** `hash_stmt` hashed `format!("{:?}", stmt)`
   including spans, so inserting a blank line dirtied every function below
   it. Hash structure, never positions.
3. **No artifact store.** Nothing persisted object code keyed by those
   hashes, and the LLVM pipeline compiles the whole module monolithically;
   per-function reuse needs per-function (or per-SCC) codegen units and a
   link step that composes cached objects.

Bar to clear before building it at all: compile times are ~64 ms for a
small file and ~1 s for an app. An incremental scheme must beat cold
compiles on real edits including its own hashing/IO overhead, and must be
byte-for-byte identical to a cold build (differential-tested), or it ships
as a footgun.
