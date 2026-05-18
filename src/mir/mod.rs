//! Mid-level IR in SSA form.
//!
//! Sits between HIR and LLVM IR, providing a Jinn-owned SSA-form
//! representation for dataflow analysis and classical optimizations.

pub mod lower;
pub mod opt;
pub mod printer;

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::ast::{FnAttrs, Span};
use crate::hir::DefId;
use crate::intern::Symbol;
use crate::types::Type;

/// A unique identifier for an SSA value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueId(pub u32);

/// A unique identifier for a basic block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

/// A MIR function in SSA form.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: Symbol,
    pub def_id: DefId,
    pub params: Vec<Param>,
    pub ret_ty: Type,
    pub blocks: Vec<BasicBlock>,
    pub entry: BlockId,
    pub span: Span,
    pub next_value: u32,
    pub next_block: u32,
    pub attrs: FnAttrs,
    /// Side-table populated by Perceus transforms. Empty until
    /// `crate::perceus::run` mutates the function.
    pub perceus: PerceusMeta,
}

/// Per-function Perceus annotations attached after the perceus passes run.
///
/// Drop instructions for trivially-droppable values are *physically deleted*
/// by the elision pass, so this side-table only encodes information that
/// codegen cannot recover from the IR alone:
///   - which non-trivial Drop should save its heap pointer for a subsequent
///     allocation (`reuse_save`),
///   - which allocation site should consume a saved slot (`reuse_consume`),
///   - the slot identifier shared between the two,
///   - which Drops form a fusion run that codegen should batch.
#[derive(Debug, Clone, Default)]
pub struct PerceusMeta {
    /// Drop instruction's *operand* ValueId → reuse slot id. When codegen
    /// processes `Drop v` and `v` is in this map, it stashes the heap pointer
    /// in slot `reuse_save[&v]` instead of releasing the allocation.
    pub reuse_save: HashMap<ValueId, u32>,
    /// Allocation instruction's *dest* ValueId → reuse slot id. When codegen
    /// processes the producer of `dest` and the dest is in this map, it tries
    /// to consume the slot first; on miss it falls back to malloc.
    pub reuse_consume: HashMap<ValueId, u32>,
    /// Drop instructions that are part of a fused run. The Vec elements are
    /// the operand ValueIds of consecutive Drop instructions; codegen emits a
    /// single batched free for the whole run.
    pub drop_fusion_runs: Vec<Vec<ValueId>>,
    /// Allocation instructions whose result reuses the storage of an incoming
    /// owned parameter (tail-call reuse). Maps alloc dest → param ValueId.
    pub tail_reuse: HashMap<ValueId, ValueId>,
    /// Allocation sites that occur inside a loop body and would benefit from a
    /// pool. Currently informational only — wired into stats reporting.
    pub pool_allocs: HashSet<ValueId>,
    /// RcInc instructions whose operand is single-use, non-escaping, and so
    /// can be promoted to a move (delete the inc, the matching dec is also
    /// dropped by the elision pass).
    pub borrow_to_move: HashSet<ValueId>,
    /// Slot ids that should use *Vec semantics* instead of the default Rc
    /// semantics. For Vec slots, the Drop save path deep-drops elements but
    /// preserves the {data, len, cap} header (and its data buffer); the
    /// matching consume path resets `len = 0`, leaving the data buffer
    /// available for the next push run, eliminating both header and buffer
    /// mallocs across a loop iteration.
    pub vec_slots: HashSet<u32>,
}

/// Aggregate statistics across all functions, surfaced via `--debug-perceus`.
#[derive(Debug, Clone, Default)]
pub struct PerceusStats {
    pub functions_analyzed: u32,
    pub bindings_analyzed: u32,
    pub drops_elided: u32,
    pub drops_sunk: u32,
    pub drops_fused: u32,
    pub reuse_pairs: u32,
    pub borrows_promoted: u32,
    pub fbip_sites: u32,
    pub tail_reuse_sites: u32,
    pub pool_hints: u32,
}

impl Function {
    pub fn new_value(&mut self) -> ValueId {
        let id = ValueId(self.next_value);
        self.next_value += 1;
        id
    }

    pub fn new_block(&mut self, label: &str) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        self.blocks.push(BasicBlock {
            id,
            label: Symbol::intern(&format!("{}{}", label, id.0)),
            phis: Vec::new(),
            insts: Vec::new(),
            terminator: Terminator::Unreachable,
        });
        id
    }

    pub fn block(&self, id: BlockId) -> &BasicBlock {
        self.blocks
            .iter()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("ICE: MIR block {} not found — possible compiler bug", id))
    }

    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock {
        self.blocks
            .iter_mut()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("ICE: MIR block {} not found — possible compiler bug", id))
    }

    /// Build predecessor map for all blocks.
    pub fn predecessors(&self) -> HashMap<BlockId, Vec<BlockId>> {
        let mut preds: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
        for bb in &self.blocks {
            preds.entry(bb.id).or_default();
            for succ in bb.terminator.successors() {
                preds.entry(succ).or_default().push(bb.id);
            }
        }
        preds
    }
}

#[derive(Debug, Clone)]
pub struct Param {
    pub value: ValueId,
    pub name: Symbol,
    pub ty: Type,
}

/// A basic block containing phi nodes, instructions, and a terminator.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub label: Symbol,
    pub phis: Vec<Phi>,
    pub insts: Vec<Instruction>,
    pub terminator: Terminator,
}

/// A phi node at the start of a basic block.
#[derive(Debug, Clone)]
pub struct Phi {
    pub dest: ValueId,
    pub ty: Type,
    pub incoming: Vec<(BlockId, ValueId)>,
}

/// An SSA instruction.
#[derive(Debug, Clone)]
pub struct Instruction {
    pub dest: Option<ValueId>,
    pub kind: InstKind,
    pub ty: Type,
    pub span: Span,
    /// Optional HIR DefId for Perceus hint threading.
    pub def_id: Option<DefId>,
}

/// MIR instruction kinds.
#[derive(Debug, Clone)]
pub enum InstKind {
    IntConst(i64),
    FloatConst(f64),
    BoolConst(bool),
    StringConst(String),
    Void,

    BinOp(BinOp, ValueId, ValueId),
    UnaryOp(UnaryOp, ValueId),
    Cmp(CmpOp, ValueId, ValueId, Type),

    Call(Symbol, Vec<ValueId>),
    /// Method-call dispatch. The trailing `bool` is the "borrow" flag: when
    /// `true`, codegen for known short-lived-read methods (`vec.get`,
    /// `vec.first`, `vec.last`) skips the otherwise-mandatory deep clone of
    /// the returned heap value. Set only by `lower_stmt(Bind)` when the
    /// destination binding was demoted to `Ownership::Borrowed` by
    /// `escape::apply_demotions` (T1 reads of clonable types). For methods
    /// whose codegen does not clone anyway (`map.get`, `set` ops, `pq`/`deque`
    /// peek), the flag is a no-op but harmless.
    MethodCall(ValueId, Symbol, Vec<ValueId>, bool),
    IndirectCall(ValueId, Vec<ValueId>),

    Load(Symbol),
    Store(Symbol, ValueId),

    FieldGet(ValueId, Symbol),
    FieldSet(ValueId, Symbol, ValueId),
    /// Direct field store into a named variable's alloca (for mem_vars).
    FieldStore(Symbol, Symbol, ValueId),
    /// Tombstone a struct field in a mem_var: store the LLVM zero-init for
    /// the field's type at its slot. Emitted by P4 §5.2 Perceus partial-move
    /// (`x is take y.field` semantics) so the parent's scope-exit drop loads
    /// a null/zero field and skips the (already-moved) heap allocation.
    /// Null-safety of the field's drop is required (all jinn heap types).
    FieldTombstone(Symbol, Symbol),

    Index(ValueId, ValueId),
    /// Bounds-proven indexing (e.g., compiler-generated foreach loops).
    /// Codegen may skip dynamic bounds checks for this instruction.
    IndexUnchecked(ValueId, ValueId),
    IndexSet(ValueId, ValueId, ValueId),
    /// Direct index store into a named variable's alloca (for mem_var arrays).
    IndexStore(Symbol, ValueId, ValueId),

    StructInit(Symbol, Vec<(Symbol, ValueId)>),
    VariantInit(Symbol, Symbol, u32, Vec<ValueId>),
    ArrayInit(Vec<ValueId>),

    Cast(ValueId, Type),
    StrictCast(ValueId, Type),
    Ref(ValueId),
    Deref(ValueId),

    Alloc(ValueId),
    Drop(ValueId, Type),
    /// Fused drop: a run of >=2 adjacent drops coalesced by the Perceus
    /// drop-fusion pass. Codegen emits a single batched runtime call rather
    /// than N individual drop sequences.
    DropMany(Vec<(ValueId, Type)>),
    RcInc(ValueId),
    RcDec(ValueId),

    /// Copy/move — eliminated by copy propagation.
    Copy(ValueId),

    /// Deep clone of a heap-owned value. Inserted by the typer / MIR
    /// lowering at field-access escape boundaries (auto-copy) and at
    /// explicit `copy` modifier sites. Produces a fresh-owned value
    /// of the given type; codegen lowers via `Compiler::clone_value`.
    /// Unlike `Copy`, this is *not* an SSA-only renaming — it has
    /// runtime effect (allocation, refcount bumps, deep traversal).
    Clone(ValueId, Type),

    /// Reference to a named top-level function, used as a first-class value.
    FnRef(Symbol),

    Slice(ValueId, ValueId, ValueId),

    // ── Collections (needed for fusion/deforestation optimization) ──
    VecNew(Vec<ValueId>),
    VecPush(ValueId, ValueId),
    VecLen(ValueId),
    MapInit,

    // ── Closures (needed for escape analysis) ──
    ClosureCreate(Symbol, Vec<ValueId>),
    ClosureCall(ValueId, Vec<ValueId>),

    // ── RC (needed for Perceus on MIR) ──
    RcNew(ValueId, Type),
    RcClone(ValueId),

    // ── Actors/channels (needed for actor optimization pass) ──
    SpawnActor(Symbol, Vec<(Symbol, ValueId)>),
    ChanCreate(Type, Option<ValueId>),
    ChanSend(ValueId, ValueId),
    ChanRecv(ValueId),
    SelectArm(Vec<ValueId>, bool),

    // ── Builtins (needed so MIR can fold/eliminate them) ──
    Log(ValueId),
    Assert(ValueId, String),

    // ── Inline assembly ──
    InlineAsm(String, Vec<ValueId>),

    // ── Global variables ──
    GlobalLoad(Symbol),
    GlobalStore(Symbol, ValueId),
}

/// Binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Ushr,
    And,
    Or,
}

/// Unary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

/// Comparison operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

/// Block terminators.
#[derive(Debug, Clone)]
pub enum Terminator {
    Goto(BlockId),
    Branch(ValueId, BlockId, BlockId),
    Return(Option<ValueId>),
    Switch(ValueId, Vec<(i64, BlockId)>, BlockId),
    Unreachable,
}

impl Terminator {
    /// Return all successor block IDs.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Terminator::Goto(b) => vec![*b],
            Terminator::Branch(_, t, f) => vec![*t, *f],
            Terminator::Return(_) => vec![],
            Terminator::Switch(_, cases, default) => {
                let mut succs: Vec<BlockId> = cases.iter().map(|(_, b)| *b).collect();
                succs.push(*default);
                succs
            }
            Terminator::Unreachable => vec![],
        }
    }

    /// Replace all occurrences of old_block with new_block in successors.
    pub fn replace_successor(&mut self, old: BlockId, new: BlockId) {
        match self {
            Terminator::Goto(b) => {
                if *b == old {
                    *b = new;
                }
            }
            Terminator::Branch(_, t, f) => {
                if *t == old {
                    *t = new;
                }
                if *f == old {
                    *f = new;
                }
            }
            Terminator::Switch(_, cases, default) => {
                for (_, b) in cases.iter_mut() {
                    if *b == old {
                        *b = new;
                    }
                }
                if *default == old {
                    *default = new;
                }
            }
            Terminator::Return(_) | Terminator::Unreachable => {}
        }
    }
}

/// A complete MIR program.
#[derive(Debug, Clone)]
pub struct Program {
    pub functions: Vec<Function>,
    pub types: Vec<TypeDef>,
    pub externs: Vec<ExternDecl>,
    pub globals: Vec<GlobalDef>,
}

/// Global variable definition in MIR.
#[derive(Debug, Clone)]
pub struct GlobalDef {
    pub name: Symbol,
    pub ty: Type,
}

/// Type definition in MIR.
#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: Symbol,
    pub fields: Vec<(Symbol, Type)>,
}

/// External function declaration.
#[derive(Debug, Clone)]
pub struct ExternDecl {
    pub name: Symbol,
    pub params: Vec<Type>,
    pub ret: Type,
}

impl fmt::Display for ValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Exp => "pow",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::Ushr => ">>>",
            Self::And => "and",
            Self::Or => "or",
        })
    }
}

impl fmt::Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
        })
    }
}
