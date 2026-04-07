//! Mid-level IR in SSA form.
//!
//! Sits between HIR and LLVM IR, providing a Jade-owned SSA-form
//! representation for dataflow analysis and classical optimizations.

pub mod lower;
pub mod opt;
pub mod printer;

use std::collections::HashMap;
use std::fmt;

use crate::ast::Span;
use crate::hir::DefId;
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
    pub name: String,
    pub def_id: DefId,
    pub params: Vec<Param>,
    pub ret_ty: Type,
    pub blocks: Vec<BasicBlock>,
    pub entry: BlockId,
    pub span: Span,
    pub next_value: u32,
    pub next_block: u32,
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
            label: label.to_string(),
            phis: Vec::new(),
            insts: Vec::new(),
            terminator: Terminator::Unreachable,
        });
        id
    }

    pub fn block(&self, id: BlockId) -> &BasicBlock {
        self.blocks.iter().find(|b| b.id == id)
            .unwrap_or_else(|| panic!("block {} not found", id))
    }

    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock {
        self.blocks.iter_mut().find(|b| b.id == id)
            .unwrap_or_else(|| panic!("block {} not found", id))
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
    pub name: String,
    pub ty: Type,
}

/// A basic block containing phi nodes, instructions, and a terminator.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub label: String,
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

    Call(String, Vec<ValueId>),
    MethodCall(ValueId, String, Vec<ValueId>),
    IndirectCall(ValueId, Vec<ValueId>),

    Load(String),
    Store(String, ValueId),

    FieldGet(ValueId, String),
    FieldSet(ValueId, String, ValueId),
    /// Direct field store into a named variable's alloca (for mem_vars).
    FieldStore(String, String, ValueId),

    Index(ValueId, ValueId),
    IndexSet(ValueId, ValueId, ValueId),
    /// Direct index store into a named variable's alloca (for mem_var arrays).
    IndexStore(String, ValueId, ValueId),

    StructInit(String, Vec<(String, ValueId)>),
    VariantInit(String, String, u32, Vec<ValueId>),
    ArrayInit(Vec<ValueId>),

    Cast(ValueId, Type),
    StrictCast(ValueId, Type),
    Ref(ValueId),
    Deref(ValueId),

    Alloc(ValueId),
    Drop(ValueId, Type),
    RcInc(ValueId),
    RcDec(ValueId),

    /// Copy/move — eliminated by copy propagation.
    Copy(ValueId),

    /// Reference to a named top-level function, used as a first-class value.
    FnRef(String),

    Slice(ValueId, ValueId, ValueId),

    // ── Collections (needed for fusion/deforestation optimization) ──
    VecNew(Vec<ValueId>),
    VecPush(ValueId, ValueId),
    VecLen(ValueId),
    MapInit,
    SetInit,
    PQInit,
    DequeInit,

    // ── Closures (needed for escape analysis) ──
    ClosureCreate(String, Vec<ValueId>),
    ClosureCall(ValueId, Vec<ValueId>),

    // ── RC (needed for Perceus on MIR) ──
    RcNew(ValueId, Type),
    RcClone(ValueId),
    WeakUpgrade(ValueId),

    // ── Actors/channels (needed for actor optimization pass) ──
    SpawnActor(String, Vec<ValueId>),
    ChanCreate(Type, Option<ValueId>),
    ChanSend(ValueId, ValueId),
    ChanRecv(ValueId),
    SelectArm(Vec<ValueId>, bool),

    // ── Builtins (needed so MIR can fold/eliminate them) ──
    Log(ValueId),
    Assert(ValueId, String),

    // ── Dynamic dispatch ──
    DynDispatch(ValueId, String, String, Vec<ValueId>),
    /// Box a concrete value into a fat pointer {data_ptr, vtable_ptr} for dyn trait.
    DynCoerce(ValueId, String, String),

    // ── Inline assembly ──
    InlineAsm(String, Vec<ValueId>),
}

/// Binary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod, Exp,
    BitAnd, BitOr, BitXor, Shl, Shr,
    And, Or,
}

/// Unary operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg, Not, BitNot,
}

/// Comparison operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq, Ne, Lt, Gt, Le, Ge,
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
            Terminator::Goto(b) => { if *b == old { *b = new; } }
            Terminator::Branch(_, t, f) => {
                if *t == old { *t = new; }
                if *f == old { *f = new; }
            }
            Terminator::Switch(_, cases, default) => {
                for (_, b) in cases.iter_mut() {
                    if *b == old { *b = new; }
                }
                if *default == old { *default = new; }
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
}

/// Type definition in MIR.
#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: String,
    pub fields: Vec<(String, Type)>,
}

/// External function declaration.
#[derive(Debug, Clone)]
pub struct ExternDecl {
    pub name: String,
    pub params: Vec<Type>,
    pub ret: Type,
}

impl fmt::Display for ValueId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "v{}", self.0) }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "bb{}", self.0) }
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Add => "+", Self::Sub => "-", Self::Mul => "*",
            Self::Div => "/", Self::Mod => "%", Self::Exp => "pow",
            Self::BitAnd => "&", Self::BitOr => "|", Self::BitXor => "^",
            Self::Shl => "<<", Self::Shr => ">>",
            Self::And => "and", Self::Or => "or",
        })
    }
}

impl fmt::Display for CmpOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Eq => "==", Self::Ne => "!=",
            Self::Lt => "<",  Self::Gt => ">",
            Self::Le => "<=", Self::Ge => ">=",
        })
    }
}
