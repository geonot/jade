pub mod lower;
pub mod opt;
pub mod printer;
pub mod verify;

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::ast::{FnAttrs, Span};
use crate::hir::DefId;
use crate::intern::Symbol;
use crate::types::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValueId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

/// Canonical LLVM symbol name for an actor handler function. Used by BOTH
/// MIR lowering (which emits the handler as a `Function`) and the actor
/// codegen driver (which looks it up to emit a call from the mailbox/dispatch
/// loop). The actor name + handler name (or the `loop` marker) is unique, so
/// the resulting symbol never collides with user functions.
pub fn actor_handler_fn_name(actor: Symbol, handler: &crate::hir::HandlerDef) -> String {
    if handler.is_loop {
        format!("__actor_{}_loop", actor)
    } else {
        format!("__actor_{}_h_{}", actor, handler.name)
    }
}

/// Canonical LLVM symbol name for an actor's field-default initializer
/// function. MIR lowering emits a `void(ptr state)` `Function` under this name
/// that populates the actor state struct with its field defaults; the codegen
/// actor factory calls it before applying user-supplied spawn overrides.
pub fn actor_init_fn_name(actor: Symbol) -> String {
    format!("__actor_init_{}", actor)
}

/// Canonical LLVM symbol name for an actor's loop-sleep-duration function. When
/// a `loop` handler declares a sleep interval (`loop ... sleep <expr>`), MIR
/// lowering emits an `i64(ptr state)` `Function` under this name that evaluates
/// the duration expression against the actor state; the codegen actor loop
/// calls it each iteration to obtain the millisecond count.
pub fn actor_sleep_fn_name(actor: Symbol) -> String {
    format!("__actor_sleep_{}", actor)
}

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

    /// True if this function is a coroutine/generator body. Such functions are
    /// lowered like any other (params = captures, body lowered normally with
    /// `yield` → `__yield` calls), but codegen gives them a fixed
    /// `void(ptr gen_ptr)` ABI: the single LLVM parameter is the generator
    /// struct pointer, captures are loaded from it in the prologue, and a
    /// `Return` terminator marks the generator done and suspends instead of
    /// returning normally.
    pub is_coroutine: bool,

    pub perceus: PerceusMeta,
}

#[derive(Debug, Clone, Default)]
pub struct PerceusMeta {
    pub reuse_save: HashMap<ValueId, u32>,

    pub reuse_consume: HashMap<ValueId, u32>,

    pub drop_fusion_runs: Vec<Vec<ValueId>>,

    pub tail_reuse: HashMap<ValueId, ValueId>,

    pub vec_slots: HashSet<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct PerceusStats {
    pub functions_analyzed: u32,
    pub bindings_analyzed: u32,
    pub drops_elided: u32,
    pub drops_sunk: u32,
    pub drops_fused: u32,
    pub reuse_pairs: u32,
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
    pub ownership: crate::hir::Ownership,
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub label: Symbol,
    pub phis: Vec<Phi>,
    pub insts: Vec<Instruction>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone)]
pub struct Phi {
    pub dest: ValueId,
    pub ty: Type,
    pub incoming: Vec<(BlockId, ValueId)>,
}

#[derive(Debug, Clone)]
pub struct Instruction {
    pub dest: Option<ValueId>,
    pub kind: InstKind,
    pub ty: Type,
    pub span: Span,

    pub def_id: Option<DefId>,
}

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

    /// Runtime-system call (store/KV/Vec/FTS/Graph/TS/Bloom/atomic/channel
    /// lifecycle/transaction). Same calling convention as `Call`, but tagged
    /// so the verifier and Perceus can apply runtime-specific ownership and
    /// effect rules. The `Symbol` is the runtime function name; the prefix
    /// (e.g. `__store_`, `__kv_`, `__atomic_`) identifies the category.
    RuntimeOp(Symbol, Vec<ValueId>),

    MethodCall(ValueId, Symbol, Vec<ValueId>, bool),
    IndirectCall(ValueId, Vec<ValueId>),

    Load(Symbol),
    Store(Symbol, ValueId),

    FieldGet(ValueId, Symbol),
    FieldSet(ValueId, Symbol, ValueId),

    FieldStore(Symbol, Symbol, ValueId),

    /// SSA-form field zeroing: takes a struct ValueId, returns a new struct
    /// ValueId identical to the input except that `field` is replaced with
    /// the zero value of its type. Used by `take` of a non-trivially-
    /// droppable field so the parent's eventual drop sees a null/zero slot
    /// and skips that field's drop.
    FieldClear(ValueId, Symbol),

    Index(ValueId, ValueId),

    IndexUnchecked(ValueId, ValueId),
    IndexSet(ValueId, ValueId, ValueId),

    IndexStore(Symbol, ValueId, ValueId),

    StructInit(Symbol, Vec<(Symbol, ValueId)>),
    VariantInit(Symbol, Symbol, u32, Vec<ValueId>),
    ArrayInit(Vec<ValueId>),

    /// `Cast(val, src_ty, dst_ty)` — convert `val` (typed `src_ty`) to `dst_ty`.
    /// Carrying both types is required so codegen can pick zext vs sext
    /// correctly (Jinn signedness is not recoverable from LLVM types alone, and
    /// in particular `bool` (i1) must always zero-extend).
    Cast(ValueId, Type, Type),
    StrictCast(ValueId, Type, Type),
    Ref(ValueId),
    Deref(ValueId),

    Alloc(ValueId),
    Drop(ValueId, Type),

    DropMany(Vec<(ValueId, Type)>),

    Copy(ValueId),

    Clone(ValueId, Type),

    FnRef(Symbol),

    Slice(ValueId, ValueId, ValueId),

    VecNew(Vec<ValueId>),
    VecPush(ValueId, ValueId),
    VecLen(ValueId),
    MapInit,

    ClosureCreate(Symbol, Vec<ValueId>),
    ClosureCall(ValueId, Vec<ValueId>),

    SpawnActor(Symbol, Vec<(Symbol, ValueId)>),
    ChanCreate(Type, Option<ValueId>),
    ChanSend(ValueId, ValueId),
    ChanRecv(ValueId),
    SelectArm(Vec<ValueId>, bool),

    Log(ValueId),
    Assert(ValueId, String),

    InlineAsm(String, Vec<ValueId>),

    GlobalLoad(Symbol),
    GlobalStore(Symbol, ValueId),
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Goto(BlockId),
    Branch(ValueId, BlockId, BlockId),
    Return(Option<ValueId>),
    Switch(ValueId, Vec<(i64, BlockId)>, BlockId),
    Unreachable,
}

impl Terminator {
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

#[derive(Debug, Clone)]
pub struct Program {
    pub functions: Vec<Function>,
    pub types: Vec<TypeDef>,
    pub externs: Vec<ExternDecl>,
    pub globals: Vec<GlobalDef>,
}

#[derive(Debug, Clone)]
pub struct GlobalDef {
    pub name: Symbol,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: Symbol,
    pub fields: Vec<(Symbol, Type)>,
}

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
