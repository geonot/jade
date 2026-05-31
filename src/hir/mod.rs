use crate::ast::{self, Span};
use crate::intern::Symbol;
use crate::types::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(pub u32);

impl DefId {
    pub const BUILTIN: DefId = DefId(0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Owned,

    Borrowed,

    BorrowMut,

    Raw,
}

impl Default for Ownership {
    fn default() -> Self {
        Ownership::Owned
    }
}

impl Ownership {
    pub fn is_borrow(self) -> bool {
        matches!(self, Ownership::Borrowed | Ownership::BorrowMut)
    }

    pub fn is_mutable_alias(self) -> bool {
        matches!(self, Ownership::BorrowMut)
    }
}

impl std::fmt::Display for Ownership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ownership::Owned => f.write_str("owned"),
            Ownership::Borrowed => f.write_str("&"),
            Ownership::BorrowMut => f.write_str("&mut"),
            Ownership::Raw => f.write_str("*raw"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Global {
    pub name: Symbol,
    pub init: Expr,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<Fn>,
    pub types: Vec<TypeDef>,
    pub enums: Vec<EnumDef>,
    pub externs: Vec<ExternFn>,
    pub err_defs: Vec<ErrDef>,
    pub actors: Vec<ActorDef>,
    pub stores: Vec<StoreDef>,
    pub trait_impls: Vec<TraitImpl>,
    pub supervisors: Vec<SupervisorDef>,
    pub type_aliases: Vec<(Symbol, Type, Span)>,
    pub newtypes: Vec<(Symbol, Type, Span)>,
    pub migrations: Vec<crate::ast::MigrationDef>,
    pub globals: Vec<Global>,
}

#[derive(Debug, Clone)]
pub struct TraitImpl {
    pub trait_name: Option<Symbol>,
    pub trait_type_args: Vec<Type>,
    pub type_name: Symbol,
    pub methods: Vec<Fn>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Fn {
    pub def_id: DefId,
    pub name: Symbol,
    pub params: Vec<Param>,
    pub ret: Type,

    pub error_types: Vec<Type>,
    pub body: Block,
    pub span: Span,
    pub generic_origin: Option<Symbol>,
    pub is_generator: bool,
    pub attrs: crate::ast::FnAttrs,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub def_id: DefId,
    pub name: Symbol,
    pub ty: Type,
    pub ownership: Ownership,

    pub access_mod: Option<crate::ast::AccessMod>,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub fields: Vec<Field>,
    pub methods: Vec<Fn>,
    pub layout: crate::ast::LayoutAttrs,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: Symbol,
    pub ty: Type,
    pub default: Option<Expr>,

    pub access_mod: Option<crate::ast::AccessMod>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub variants: Vec<Variant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: Symbol,
    pub fields: Vec<VField>,
    pub tag: u32,
    pub discriminant: Option<i64>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VField {
    pub name: Option<Symbol>,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct ExternFn {
    pub def_id: DefId,
    pub name: Symbol,
    pub params: Vec<(Symbol, Type)>,
    pub ret: Type,
    pub variadic: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ErrDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub variants: Vec<ErrVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ErrVariant {
    pub name: Symbol,
    pub fields: Vec<Type>,
    pub tag: u32,
    pub span: Span,
}

pub type Block = Vec<Stmt>;

#[derive(Debug, Clone)]
pub struct ActorDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub fields: Vec<Field>,

    pub field_def_ids: Vec<DefId>,
    pub handlers: Vec<HandlerDef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct HandlerDef {
    pub name: Symbol,
    pub params: Vec<Param>,
    pub is_loop: bool,
    pub loop_sleep_ms: Option<Expr>,
    pub body: Block,
    pub tag: u32,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StoreField {
    pub name: Symbol,
    pub ty: Type,
    pub default: Option<Expr>,
    pub decorators: Vec<ast::FieldDecorator>,
    pub is_relation: bool,
    pub is_has_many: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StoreDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub decorators: Vec<ast::StoreDecorator>,
    pub fields: Vec<StoreField>,
    pub methods: Vec<Fn>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorStrategy {
    OneForOne,
    OneForAll,
    RestForOne,
}

#[derive(Debug, Clone)]
pub struct SupervisorDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub strategy: SupervisorStrategy,
    pub children: Vec<Symbol>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StoreFilter {
    pub field: Symbol,
    pub op: BinOp,
    pub value: Expr,
    pub span: Span,
    pub extra: Vec<(ast::LogicalOp, StoreFilterCond)>,
}

#[derive(Debug, Clone)]
pub struct StoreFilterCond {
    pub field: Symbol,
    pub op: BinOp,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Bind(Bind),
    TupleBind(Vec<(DefId, Symbol, Type)>, Expr, Span),
    Assign(Expr, Expr, Span),
    Expr(Expr),
    If(If),
    While(While),
    For(For),
    Loop(Loop),
    Ret(Option<Expr>, Type, Span),
    Break(Option<Expr>, Span),
    Continue(Span),

    Nop(Span),
    Match(Match),
    Asm(AsmBlock),
    Drop(DefId, Symbol, Type, Span),
    ErrReturn(Expr, Type, Span),

    Defer(Block, Span),
    StoreInsert(Symbol, Vec<Expr>, Span),
    StoreDelete(Symbol, Box<StoreFilter>, Span),
    StoreDestroy(Symbol, Box<StoreFilter>, Span),
    StoreSet(Symbol, Vec<(Symbol, Expr)>, Box<StoreFilter>, Span),
    StoreRestore(Symbol, Box<StoreFilter>, Span),
    StoreSave(Symbol, Span),
    Transaction(Block, Span),
    ChannelClose(Expr, Span),
    Stop(Expr, Span),
    SimFor(For, Span),
    SimBlock(Block, Span),
    UseLocal(Vec<Symbol>, Option<Vec<Symbol>>, Option<Symbol>, Span),

    GlobalStore(Symbol, Expr, Span),
}

#[derive(Debug, Clone)]
pub struct SelectArm {
    pub is_send: bool,
    pub chan: Expr,
    pub value: Option<Expr>,
    pub binding: Option<Symbol>,
    pub bind_id: Option<DefId>,
    pub elem_ty: Type,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Bind {
    pub def_id: DefId,
    pub name: Symbol,
    pub value: Expr,
    pub ty: Type,
    pub ownership: Ownership,
    pub atomic: bool,

    pub access_mod: Option<crate::ast::AccessMod>,
    pub span: Span,
}

use crate::ast::{BinOp, UnaryOp};

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
    Void,
    Var(DefId, Symbol),
    FnRef(DefId, Symbol),
    VariantRef(Symbol, Symbol, u32),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
    Call(DefId, Symbol, Vec<Expr>),
    IndirectCall(Box<Expr>, Vec<Expr>),
    Builtin(BuiltinFn, Vec<Expr>),
    Method(Box<Expr>, Symbol, Symbol, Vec<Expr>),
    StringMethod(Box<Expr>, Symbol, Vec<Expr>),

    DeferredMethod(Box<Expr>, Symbol, Vec<Expr>),
    VecMethod(Box<Expr>, Symbol, Vec<Expr>),
    MapMethod(Box<Expr>, Symbol, Vec<Expr>),
    VecNew(Vec<Expr>),
    MapNew,
    Field(Box<Expr>, Symbol, usize),
    Index(Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Coerce(Box<Expr>, CoercionKind),
    Cast(Box<Expr>, Type),
    Array(Vec<Expr>),
    Tuple(Vec<Expr>),
    Struct(Symbol, Vec<FieldInit>),
    VariantCtor(Symbol, Symbol, u32, Vec<FieldInit>),
    IfExpr(Box<If>),
    Pipe(Box<Expr>, DefId, Symbol, Vec<Expr>),
    Block(Block),
    Lambda(Vec<Param>, Block),
    Ref(Box<Expr>),
    Deref(Box<Expr>),
    ListComp(
        Box<Expr>,
        DefId,
        String,
        Box<Expr>,
        Option<Box<Expr>>,
        Option<Box<Expr>>,
    ),
    Syscall(Vec<Expr>),
    Spawn(Symbol, Vec<(Symbol, Expr)>),
    Send(Box<Expr>, Symbol, Symbol, u32, Vec<Expr>),
    CoroutineCreate(Symbol, Vec<Stmt>),
    CoroutineNext(Box<Expr>),
    Yield(Box<Expr>),
    StoreQuery(Symbol, Box<StoreFilter>),
    StoreCount(Symbol),
    StoreAll(Symbol),
    StoreGet(Symbol, Box<Expr>),
    StoreFirst(Symbol, Box<StoreFilter>),
    StoreExists(Symbol, Box<StoreFilter>),
    StoreDistinct(Symbol, Symbol),
    StoreSum(Symbol, Symbol),
    StoreAvg(Symbol, Symbol),
    StoreMin(Symbol, Symbol),
    StoreMax(Symbol, Symbol),
    StoreVersionCount(Symbol, Box<Expr>),
    StoreHistory(Symbol, Box<Expr>),
    StoreAtVersion(Symbol, Box<Expr>, Box<Expr>),
    ViewCount(Symbol, Box<StoreFilter>),
    ViewAll(Symbol, Box<StoreFilter>),

    KvGet(Symbol, Box<Expr>),
    KvHas(Symbol, Box<Expr>),
    KvCount(Symbol),
    KvSet(Symbol, Box<Expr>, Box<Expr>),
    KvDel(Symbol, Box<Expr>),
    KvIncr(Symbol, Box<Expr>, Box<Expr>),

    VecNearest(Symbol, Box<Expr>, Box<Expr>),
    VecInsert(Symbol, Box<Expr>),
    VecCount(Symbol),

    BloomTest(Symbol, Symbol, Box<Expr>),

    FtsSearch(Symbol, Symbol, Box<Expr>),
    FtsCount(Symbol, Symbol),

    GraphFrom(Symbol, Box<Expr>),
    GraphTo(Symbol, Box<Expr>),

    TsLatest(Symbol),
    IterNext(Symbol, Symbol, Symbol),
    ChannelCreate(Type, Box<Expr>),
    ChannelSend(Box<Expr>, Box<Expr>),
    ChannelRecv(Box<Expr>),
    Select(Vec<SelectArm>, Option<Block>),
    Unreachable,
    StrictCast(Box<Expr>, Type),
    AsFormat(Box<Expr>, Symbol),
    AtomicLoad(Box<Expr>),
    AtomicStore(Box<Expr>, Box<Expr>),
    AtomicAdd(Box<Expr>, Box<Expr>),
    AtomicSub(Box<Expr>, Box<Expr>),
    AtomicCas(Box<Expr>, Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Box<Expr>, Box<Expr>),
    Grad(Box<Expr>),
    Einsum(Symbol, Vec<Expr>),
    Builder(Symbol, Vec<(Symbol, Expr)>),

    GeneratorCreate(DefId, Symbol, Vec<Stmt>, Vec<(Symbol, Type)>),
    GeneratorNext(Box<Expr>),

    EnumUnwrap(Box<Expr>, Symbol, u32),

    EnumIs(Box<Expr>, u32),

    GlobalLoad(Symbol),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BuiltinFn {
    Log,
    Eprint,
    Print,
    ToString,
    VolatileLoad,
    VolatileStore,
    WrappingAdd,
    WrappingSub,
    WrappingMul,
    SaturatingAdd,
    SaturatingSub,
    SaturatingMul,
    CheckedAdd,
    CheckedSub,
    CheckedMul,
    SignalHandle,
    SignalRaise,
    SignalIgnore,
    SignalDefault,
    SignalKill,
    Popcount,
    Clz,
    Ctz,
    RotateLeft,
    RotateRight,
    Bswap,
    Assert,
    ActorSpawn,
    ActorSend,
    StringFromRaw,
    StringFromPtr,
    Chr,
    GetArgs,
    Ln,
    Log2,
    Log10,
    Exp,
    Exp2,
    PowF,
    Copysign,
    Fma,
    FmtFloat,
    FmtHex,
    FmtOct,
    FmtBin,
    TimeMonotonic,
    SleepMs,
    FileExists,
    AtomicLoad,
    AtomicStore,
    AtomicAdd,
    AtomicSub,
    AtomicCas,
    CompTimeTypeOf,
    CompTimeFieldsOf,
    CompTimeSizeOf,
    CharMethod(Symbol),
    Matmul,
    RegexMatch,
    RegexFindAll,
    VecWithAlloc,
    MapWithAlloc,
    ConstantTimeEq,
    GradFn,
    Einsum,
    Likely,
    Unlikely,
    FloatMethod(Symbol),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoercionKind {
    IntWiden {
        from_bits: u32,
        to_bits: u32,
        signed: bool,
    },
    IntTrunc {
        from_bits: u32,
        to_bits: u32,
    },
    FloatWiden,
    FloatNarrow,
    IntToFloat {
        signed: bool,
    },
    FloatToInt {
        signed: bool,
    },
    BoolToInt,

    ArrayToVec {
        elem_ty: Type,
        len: u64,
    },
}

#[derive(Debug, Clone)]
pub struct If {
    pub cond: Expr,
    pub then: Block,
    pub elifs: Vec<(Expr, Block)>,
    pub els: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct While {
    pub cond: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct For {
    pub bind_id: DefId,
    pub bind: Symbol,
    pub bind_ty: Type,
    pub bind2_id: Option<DefId>,
    pub bind2: Option<Symbol>,
    pub bind2_ty: Option<Type>,
    pub iter: Expr,
    pub end: Option<Expr>,
    pub step: Option<Expr>,
    pub body: Block,
    pub label: Option<Symbol>,

    pub access_mod: Option<crate::ast::AccessMod>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Loop {
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Match {
    pub subject: Expr,
    pub arms: Vec<Arm>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub pat: Pat,
    pub guard: Option<Expr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Pat {
    Wild(Span),
    Bind(DefId, Symbol, Type, Span),
    Lit(Expr),
    Ctor(String, u32, Vec<Pat>, Span),
    Or(Vec<Pat>, Span),
    Range(Box<Expr>, Box<Expr>, Span),
    Tuple(Vec<Pat>, Span),
    Array(Vec<Pat>, Span),
}

#[derive(Debug, Clone)]
pub struct FieldInit {
    pub name: Option<Symbol>,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct AsmBlock {
    pub template: String,
    pub outputs: Vec<(String, String)>,
    pub inputs: Vec<(String, Expr)>,
    pub clobbers: Vec<String>,
    pub span: Span,
}

mod print;
pub use print::pretty_print;
