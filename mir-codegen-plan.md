# MIR Codegen Refactor Plan

## Problem Statement

Codegen currently operates on HIR (tree-structured, non-SSA) and emits LLVM IR
via inkwell. MIR (SSA, block-based, 9 optimization passes) is lowered from HIR,
optimized, optionally printed (`--emit-mir`), then **discarded**. The 9 MIR
optimization passes have zero effect on compiled output.

```
Current:   HIR → MIR → optimize → /dev/null
                ↘ codegen → LLVM IR → object

Target:    HIR → MIR → optimize → codegen → LLVM IR → object
```

---

## Phase 0: MIR Completeness Audit

Before MIR can drive codegen, the MIR instruction set and lowering must cover
every construct that HIR codegen currently handles. Gaps:

### Missing MIR Instructions (must add to `InstKind`)

| Category | Missing Instructions | HIR Codegen File |
|----------|---------------------|------------------|
| **Collections** | `VecNew`, `VecPush`, `VecLen`, `VecGet`, `VecSet`, `VecSlice` | vec.rs |
| **Collections** | `MapNew`, `MapInsert`, `MapGet`, `MapContains`, `MapRemove`, `MapLen` | map.rs |
| **Collections** | `SetNew`, `SetInsert`, `SetContains`, `SetRemove`, `SetLen` | set.rs |
| **Strings** | `StringLen`, `StringSlice`, `StringConcat`, `StringFind`, `StringReplace` | string_ops.rs, string_transform.rs |
| **Strings** | `StringSplit`, `StringTrim`, `StringUpper`, `StringLower`, `StringStartsWith`, `StringEndsWith` | string_transform.rs |
| **Actors** | `SpawnActor`, `SendMsg`, `ActorSelf` | actors.rs |
| **Channels** | `ChanCreate`, `ChanSend`, `ChanRecv`, `Select` | channels.rs |
| **Coroutines** | `CoroCreate`, `CoroResume`, `Yield` | coroutines.rs |
| **RC** | `RcNew`, `RcClone`, `WeakUpgrade`, `WeakDowngrade` | rc.rs |
| **Stores** | `StoreInsert`, `StoreQuery`, `StoreUpdate`, `StoreDelete`, `StoreFilter` | stores.rs, store_ops.rs, store_filter.rs |
| **Closures** | `ClosureCreate`, `ClosureCall` | call.rs |
| **Builtins** | `Log`, `ToString`, `TypeName`, `Assert` | builtins.rs |
| **Atomics** | `AtomicLoad`, `AtomicStore`, `AtomicAdd`, `AtomicSub`, `AtomicCas` | (HIR ExprKind variants) |
| **Tuples** | `TupleInit`, `TupleGet` | (currently mapped to ArrayInit/Index) |

**Decision**: Not all need dedicated InstKind variants. Many can stay as
`Call("__runtime_fn", args)` or `MethodCall(obj, "method", args)`. The key
question per instruction: *does MIR need to reason about it for optimization,
or is it opaque?*

**Rule**: Add a dedicated MIR instruction only if:
1. An MIR optimization pass needs to inspect/transform it, OR
2. Codegen needs to distinguish it from a generic call

Everything else stays as `Call`/`MethodCall` — the MIR lowerer already does
this for spawn, channel, and atomic operations.

### Required MIR Instruction Additions (minimal set)

```rust
// Collections — needed for fusion/deforestation optimization
VecNew(Vec<ValueId>),          // vec literal
VecPush(ValueId, ValueId),     // vec.push(val)
VecLen(ValueId),               // vec.len()
MapNew,                        // empty map
SetNew,                        // empty set

// Closures — needed for escape analysis
ClosureCreate(String, Vec<ValueId>),  // captures
ClosureCall(ValueId, Vec<ValueId>),   // indirect via closure

// RC — needed for Perceus on MIR
RcNew(ValueId, Type),          // allocate Rc
RcClone(ValueId),              // increment refcount
WeakUpgrade(ValueId),          // weak → Option<Rc>

// Actors/channels — needed for actor optimization pass
SpawnActor(String, Vec<ValueId>),
ChanCreate(Type),
ChanSend(ValueId, ValueId),
ChanRecv(ValueId),
SelectArm(Vec<ValueId>),       // select across channels

// Builtins (needed so MIR can fold/eliminate them)
Log(ValueId),                  // log() — can be DCE'd if side-effect-free mode
Assert(ValueId, String),       // assert — can be stripped in release
```

### Required MIR Lowering Additions (`lower.rs`)

Current `lower_expr` handles ~25 of ~45 HIR ExprKind variants. Missing:

| HIR ExprKind | Lowering Strategy |
|-------------|-------------------|
| `Lambda` | `ClosureCreate` with captured var list |
| `Pipe` | Desugar into chained `Call` instructions |
| `ListComp` | Desugar into loop + `VecNew` + `VecPush` |
| `Builder` | Chain of `MethodCall` instructions |
| `VecNew` | `VecNew(elems)` |
| `MapNew` | `MapNew` |
| `SetNew` | `SetNew` |
| `VecMethod` | `MethodCall` or dedicated inst |
| `MapMethod` | `MethodCall` or dedicated inst |
| `StringMethod` | `MethodCall` or dedicated inst |
| `Spawn` | `SpawnActor` |
| `ChannelCreate` | `ChanCreate` |
| `ChannelSend` | `ChanSend` |
| `ChannelRecv` | `ChanRecv` |
| `Select` | `SelectArm` |
| `AtomicLoad/Store/Add/Sub/Cas` | `Call("__atomic_*")` |
| `Einsum` | `Call("__einsum")` |
| `Grad` | `Call("__grad")` |
| `ErrReturn` | Branch + Return |
| `Query` | `Call("__store_query")` |
| `Receive` | `Call("__actor_recv")` |
| `DispatchBlock` | Lower arms to blocks + Switch |
| `RangeExpr` | `StructInit("Range", ...)` |

---

## Phase 1: MIR Codegen Backend (`src/codegen/mir_codegen.rs`)

Create a new codegen path that reads MIR instead of HIR. This coexists with the
HIR codegen; a `--mir-codegen` flag selects it.

### Architecture

```
MirCodegen {
    ctx: &Context,
    module: Module,
    bld: Builder,
    
    // MIR → LLVM mappings
    value_map: HashMap<ValueId, BasicValueEnum>,   // SSA val → LLVM val
    block_map: HashMap<BlockId, BasicBlock>,        // MIR block → LLVM block
    fn_map: HashMap<String, FunctionValue>,         // declared functions
    
    // Inherited from current codegen
    structs: HashMap<String, Vec<(String, Type)>>,
    enums: HashMap<String, Vec<(String, Vec<Type>)>>,
    vtables: HashMap<(String, String), GlobalValue>,
    hints: PerceusHints,                             // Perceus still works
    reuse_tokens: HashMap<DefId, PointerValue>,
}
```

### Per-Function Compilation

```rust
fn compile_mir_fn(&mut self, func: &mir::Function) {
    // 1. Create LLVM function with parameter types
    let llvm_fn = self.declare_fn(func);
    
    // 2. Create all LLVM basic blocks upfront
    for bb in &func.blocks {
        let llvm_bb = self.ctx.append_basic_block(llvm_fn, &bb.label);
        self.block_map.insert(bb.id, llvm_bb);
    }
    
    // 3. Map parameters to ValueIds
    for (i, param) in func.params.iter().enumerate() {
        let llvm_val = llvm_fn.get_nth_param(i as u32).unwrap();
        self.value_map.insert(param.value, llvm_val);
    }
    
    // 4. Emit each block
    for bb in &func.blocks {
        self.bld.position_at_end(self.block_map[&bb.id]);
        
        // 4a. Emit phi nodes
        for phi in &bb.phis {
            let llvm_phi = self.bld.build_phi(self.llvm_ty(&phi.ty), "phi");
            self.value_map.insert(phi.dest, llvm_phi.as_basic_value());
            // Incoming edges added after all blocks processed
        }
        
        // 4b. Emit instructions
        for inst in &bb.insts {
            let val = self.emit_inst(inst);
            if let (Some(dest), Some(v)) = (inst.dest, val) {
                self.value_map.insert(dest, v);
            }
        }
        
        // 4c. Emit terminator
        self.emit_terminator(&bb.terminator);
    }
    
    // 5. Wire up phi incoming edges (needs all blocks emitted first)
    for bb in &func.blocks {
        for phi in &bb.phis {
            let llvm_phi = self.value_map[&phi.dest].into_phi_value();
            for (pred_bb, val) in &phi.incoming {
                llvm_phi.add_incoming(&[
                    (&self.value_map[val], self.block_map[pred_bb])
                ]);
            }
        }
    }
}
```

### Instruction Emission

```rust
fn emit_inst(&mut self, inst: &Instruction) -> Option<BasicValueEnum> {
    match &inst.kind {
        IntConst(n) => Some(i64_type.const_int(*n as u64, true).into()),
        FloatConst(f) => Some(f64_type.const_float(*f).into()),
        BoolConst(b) => Some(i1_type.const_int(*b as u64, false).into()),
        StringConst(s) => Some(self.build_string(s).into()),
        
        BinOp(op, l, r) => {
            let lv = self.val(*l); let rv = self.val(*r);
            Some(self.emit_binop(op, lv, rv, &inst.ty))
        }
        
        Call(name, args) => {
            let fn_val = self.fn_map[name];
            let llvm_args: Vec<_> = args.iter().map(|a| self.val(*a).into()).collect();
            self.bld.build_call(fn_val, &llvm_args, "call").try_as_basic_value().left()
        }
        
        FieldGet(obj, field) => { /* GEP into struct */ }
        FieldSet(obj, field, val) => { /* GEP + store */ }
        Index(arr, idx) => { /* GEP into array/vec */ }
        
        StructInit(name, fields) => { /* alloca + field stores */ }
        VariantInit(enum_name, variant, tag, args) => { /* tag + payload */ }
        
        Alloc(val) => { /* malloc or reuse token */ }
        Drop(val, ty) => { /* free or elide via Perceus */ }
        RcInc(val) => { /* refcount++ */ }
        RcDec(val) => { /* refcount--, maybe free */ }
        
        // ... one arm per InstKind variant
    }
}
```

### Terminator Emission

```rust
fn emit_terminator(&mut self, term: &Terminator) {
    match term {
        Goto(bb) => self.bld.build_unconditional_branch(self.block_map[bb]),
        Branch(cond, t, f) => {
            let cv = self.val(*cond).into_int_value();
            self.bld.build_conditional_branch(cv, self.block_map[t], self.block_map[f]);
        }
        Return(Some(val)) => self.bld.build_return(Some(&self.val(*val))),
        Return(None) => self.bld.build_return(None),
        Switch(val, cases, default) => {
            let v = self.val(*val).into_int_value();
            let default_bb = self.block_map[default];
            let arms: Vec<_> = cases.iter()
                .map(|(tag, bb)| (i64_type.const_int(*tag as u64, false), self.block_map[bb]))
                .collect();
            self.bld.build_switch(v, default_bb, &arms);
        }
        Unreachable => self.bld.build_unreachable(),
    }
}
```

### Perceus Integration on MIR

Perceus currently analyzes HIR. Two options:

**Option A (recommended for Phase 1)**: Keep Perceus on HIR, thread hints into
MIR via `DefId` annotations on MIR instructions. The `Instruction.span` already
carries `Span`; add an optional `DefId` to enable hint lookup.

**Option B (Phase 3)**: Port Perceus to operate on MIR. This is cleaner long-term
since MIR has explicit use-def chains in SSA form, making use counting trivial
and borrow analysis more precise.

---

## Phase 2: Jade-Specific MIR Optimizations

These are optimizations LLVM cannot perform because they require knowledge of
Jade's semantics — reference counting, collection types, actor model, etc.

### 2.1 RC Elision (`mir/opt_rc.rs`)

**What**: Eliminate unnecessary `RcInc`/`RcDec` pairs.

**Why LLVM can't**: LLVM sees opaque function calls to `rc_inc`/`rc_dec`. It
cannot prove they cancel out because it doesn't know the refcount semantics.

```
v1 = RcInc(v0)     ; inc refcount
... no other use of v0's Rc ...
v2 = RcDec(v0)     ; dec refcount
→ elide both (net-zero refcount change)
```

**Passes**:
1. **Paired inc/dec elimination**: Find `RcInc(v)` followed by `RcDec(v)` with
   no intervening escape of `v`. Remove both.
2. **Sink RcInc to use site**: If `RcInc` is far from use, move it closer to
   reduce time refcount is artificially elevated.
3. **Hoist RcDec above branch**: If both branch arms `RcDec(v)`, hoist to
   before the branch.
4. **Last-use dec fusion**: If variable's last use is immediately followed by
   `RcDec`, merge the use and dec into a "move" (consume without inc/dec).

### 2.2 Collection Operation Fusion (`mir/opt_collections.rs`)

**What**: Fuse sequences of collection operations into bulk operations.

**Why LLVM can't**: LLVM sees opaque calls to `vec_push`, `vec_get`, etc. It
cannot reason about their aggregate behavior.

```
v1 = VecNew()
v2 = VecPush(v1, a)
v3 = VecPush(v2, b)
v4 = VecPush(v3, c)
→ v4 = VecNewWithCap(3); bulk_push(v4, [a, b, c])
```

**Passes**:
1. **VecNew + N×Push → VecNewWithCap**: Pre-allocate capacity for known push
   sequences.
2. **Map/Set literal fusion**: `MapNew + N×Insert → MapFromPairs`.
3. **Redundant length checks**: `VecLen` inside loop where vec is not modified
   → hoist out of loop.
4. **Dead collection elimination**: Vec/Map/Set created, populated, but result
   never read → eliminate.

### 2.3 Actor/Channel Optimization (`mir/opt_actor.rs`)

**What**: Optimize actor spawn/message patterns.

**Why LLVM can't**: LLVM doesn't understand the actor model semantics.

**Passes**:
1. **Inline single-use actors**: Actor spawned, sent one message, never used
   again → inline the handler as a direct call.
2. **Channel direction analysis**: Channel only used for send or only for recv
   in a scope → can use simpler (lock-free) channel implementation.
3. **Dead channel elimination**: Channel created but never sent to → remove.
4. **Send/recv fusion**: `ChanSend(ch, v)` in one block immediately followed by
   `ChanRecv(ch)` in successor → direct value passing (bypass channel runtime).

### 2.4 Bounds Check Elimination (`mir/opt_bounds.rs`)

**What**: Remove redundant array/vec bounds checks.

**Why LLVM can't**: LLVM can sometimes eliminate bounds checks, but Jade's
runtime checks are opaque calls. LLVM doesn't know that `vec_get(v, i)` checks
`i < len(v)` internally.

**Passes**:
1. **Loop index bounds**: `for i in 0..vec.len()` → index `i` is in-bounds by
   construction, skip bounds check on `vec[i]`.
2. **Sequential access**: `vec[0]`, `vec[1]`, `vec[2]` when `vec.len() >= 3`
   is known → single check at start.
3. **Post-check elimination**: After `if i < vec.len()` branch taken → `vec[i]`
   is safe in that arm.

### 2.5 String Operation Fusion (`mir/opt_strings.rs`)

**What**: Fuse string concatenation chains and eliminate intermediate
allocations.

**Why LLVM can't**: String concatenation creates intermediate heap strings that
LLVM cannot see through.

```
v1 = StringConcat(a, b)
v2 = StringConcat(v1, c)
v3 = StringConcat(v2, d)
→ v3 = StringConcatMany([a, b, c, d])  // single allocation, total length known
```

**Passes**:
1. **Concat chain flattening**: N concatenations → single multi-concat with
   pre-computed total length.
2. **Format string optimization**: `toString(x) + " " + toString(y)` → single
   format call.
3. **Dead intermediate strings**: String produced by concat but immediately
   concatenated again → no intermediate allocation needed.

### 2.6 Closure Optimization (`mir/opt_closure.rs`)

**What**: Optimize closure allocation and capture.

**Why LLVM can't**: Closure environments are heap-allocated opaque structs.

**Passes**:
1. **Stack-allocate non-escaping closures**: If closure doesn't escape the
   current function (passed to `map`/`filter` that's inlined), use alloca
   instead of heap allocation for the capture struct.
2. **Capture narrowing**: If closure captures `x` but only uses `x.field`,
   capture only the field (smaller environment).
3. **Devirtualize known closures**: If indirect call target is always the same
   closure (common in HOF pipelines), convert to direct call.

---

## Phase 3: Perceus on MIR

Move Perceus analysis from HIR to MIR for better precision.

### Why MIR is Better for Perceus

| Aspect | HIR Perceus | MIR Perceus |
|--------|-------------|-------------|
| **Use counting** | Walk tree, manually track scope | SSA def-use chains: trivial |
| **Escape analysis** | Conservative: any call arg escapes | Can trace values through known calls |
| **Last-use detection** | Span-based heuristic | Precise: last instruction reading ValueId |
| **Reuse matching** | Adjacent Bind/Drop in same scope | Cross-block: value flows through phis |
| **Loop handling** | Conservative `use_count += 2` | Exact: loop back-edge analysis on CFG |

### Implementation

```rust
// New: src/perceus/mir_perceus.rs

pub fn analyze_mir(prog: &mir::Program) -> PerceusHints {
    let mut hints = PerceusHints::default();
    for func in &prog.functions {
        let use_def = build_use_def_chains(func);
        let dom_tree = build_dominator_tree(func);
        
        analyze_rc_pairs(func, &use_def, &mut hints);
        analyze_last_use_mir(func, &use_def, &mut hints);
        analyze_reuse_mir(func, &use_def, &dom_tree, &mut hints);
        analyze_closure_escape(func, &use_def, &mut hints);
    }
    hints
}
```

The SSA form makes use-def chains trivial: each `ValueId` is defined exactly
once and used 0+ times. "Use counting" is just counting occurrences of a
`ValueId` across all instructions.

---

## Phase 4: Backend Abstraction

Once codegen reads MIR, adding alternative backends becomes straightforward:

```rust
trait Backend {
    fn compile_function(&mut self, func: &mir::Function);
    fn emit_object(&self, path: &Path) -> Result<(), String>;
}

struct LlvmBackend { /* current inkwell-based codegen */ }
struct CraneliftBackend { /* faster debug builds */ }
struct CBackend { /* emit C for maximum portability */ }
```

This is a long-term goal, not part of the initial refactor.

---

## Execution Order

### Sprint 1: Foundation (MIR completeness)
1. Add missing `InstKind` variants (minimal set from Phase 0)
2. Extend `lower.rs` to handle all HIR ExprKind variants
3. Add tests: `--emit-mir` for each new construct, verify roundtrip fidelity
4. **Checkpoint**: every `.jade` test program produces valid MIR

### Sprint 2: MIR Codegen Backend
1. Create `src/codegen/mir_codegen.rs` with `MirCodegen` struct
2. Implement `emit_inst` for each `InstKind` variant
3. Implement `emit_terminator` for all terminators
4. Wire phi nodes correctly (two-pass: create, then add incoming)
5. Add `--mir-codegen` flag; keep HIR codegen as default
6. Threaded Perceus hints via DefId annotations
7. **Checkpoint**: basic programs compile via MIR codegen, output matches HIR codegen

### Sprint 3: Parity + Perceus
1. Achieve full parity: all 1018 tests pass with `--mir-codegen`
2. Port Perceus to MIR (Phase 3)
3. Remove HIR codegen, MIR codegen becomes default
4. **Checkpoint**: `--mir-codegen` flag removed, MIR is the only path

### Sprint 4: Jade-Specific Optimizations
1. RC elision (Phase 2.1) — highest impact, affects every program using Rc
2. Collection operation fusion (Phase 2.2) — affects most programs
3. Bounds check elimination (Phase 2.4) — performance-critical loops
4. String operation fusion (Phase 2.5) — common in I/O-heavy programs
5. Actor/channel optimization (Phase 2.3) — affects actor programs
6. Closure optimization (Phase 2.6) — affects HOF-heavy code
7. **Checkpoint**: benchmarks show measurable improvement over HIR codegen

---

## Risk Mitigation

| Risk | Mitigation |
|------|-----------|
| MIR lowering has semantic gaps | Extensive test suite (1018 programs); diff LLVM IR output between HIR and MIR codegen paths |
| Perceus hints break on MIR | Phase 1 keeps HIR Perceus; Phase 3 ports only after parity |
| MIR instruction set grows too large | Decision rule: dedicated inst only if MIR pass needs to reason about it; else use `Call` |
| Performance regression during transition | Both paths coexist; `--mir-codegen` flag |
| Debug info (DWARF) correctness | MIR instructions carry Span; map to debug locations during MIR codegen |

---

## Files to Create/Modify

### New Files
- `src/codegen/mir_codegen.rs` — MIR-based LLVM codegen
- `src/mir/opt_rc.rs` — RC elision optimization  
- `src/mir/opt_collections.rs` — collection operation fusion
- `src/mir/opt_bounds.rs` — bounds check elimination
- `src/mir/opt_strings.rs` — string operation fusion
- `src/mir/opt_actor.rs` — actor/channel optimization
- `src/mir/opt_closure.rs` — closure optimization
- `src/perceus/mir_perceus.rs` — Perceus analysis on MIR

### Modified Files
- `src/mir/mod.rs` — add InstKind variants, MIR Program structure
- `src/mir/lower.rs` — handle all HIR ExprKind variants
- `src/mir/opt.rs` — integrate new optimization passes
- `src/mir/printer.rs` — print new instruction kinds
- `src/codegen/mod.rs` — add MirCodegen pathway
- `src/main.rs` — add `--mir-codegen` flag, wire MIR to codegen

### Unchanged
- All 24 existing codegen files (kept for HIR path during transition)
- `src/perceus/mod.rs`, `analysis.rs`, `uses.rs` (kept until Phase 3)
- `src/hir.rs`, `src/ownership.rs` (no changes needed)
