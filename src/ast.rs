//! Abstract syntax tree produced by the parser. Pure data; no semantics.

pub use crate::intern::Symbol;
use crate::types::Type;

pub type Block = Vec<Stmt>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
    /// Optional source-file marker (interned). `None` means "unknown" or
    /// "synthesized". When `Some`, diagnostics print `file:line:col`
    /// instead of just `line L:C`, which is essential for multi-file
    /// projects where the same line/col can refer to different sources.
    pub file: Option<Symbol>,
}

impl Span {
    pub fn new(start: usize, end: usize, line: u32, col: u32) -> Self {
        Self {
            start,
            end,
            line,
            col,
            file: None,
        }
    }
    pub fn dummy() -> Self {
        Self {
            start: 0,
            end: 0,
            line: 0,
            col: 0,
            file: None,
        }
    }

    /// Builder: attach a filename to this span. Used by the lexer/parser
    /// pipeline after tokens are produced for a given source file.
    pub fn with_file(mut self, file: Symbol) -> Self {
        self.file = Some(file);
        self
    }

    /// Render as `"path/to/file.jn:LINE:COL"` when a file is known,
    /// otherwise as `"line LINE:COL"`. Use this in diagnostic messages
    /// to keep prior output format when no filename, while making
    /// multi-file projects unambiguous when a filename is set.
    pub fn loc(&self) -> String {
        match self.file {
            Some(f) => format!("{}:{}:{}", f.as_str(), self.line, self.col),
            None => format!("line {}:{}", self.line, self.col),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Decl {
    Fn(Fn),
    Type(TypeDef),
    Enum(EnumDef),
    Extern(ExternFn),
    Use(UseDecl),
    ErrDef(ErrDef),
    Test(TestBlock),
    Actor(ActorDef),
    Store(StoreDef),
    Migration(MigrationDef),
    View(ViewDef),
    Trait(TraitDef),
    Impl(ImplBlock),
    Const(Symbol, Expr, Span),
    Global(Symbol, Expr, Span),
    Supervisor(SupervisorDef),
    TypeAlias(Symbol, Type, Span),
    Newtype(Symbol, Type, Span),
    TopStmt(Stmt),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Shr,
    Ushr,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Bind(Bind),
    TupleBind(Vec<Symbol>, Expr, Span),
    Assign(Expr, Expr, Span),
    Expr(Expr),
    If(If),
    While(While),
    For(For),
    Loop(Loop),
    Ret(Option<Expr>, Span),
    Break(Option<Expr>, Span),
    Continue(Span),
    /// `nop` — a no-op statement (Python-style `pass`). Compiles to nothing.
    Nop(Span),
    Match(Match),
    Asm(AsmBlock),
    ErrReturn(Expr, Span),
    /// `defer <block>` — runs the block at function exit (any return path).
    Defer(Block, Span),
    StoreInsert(Symbol, Vec<FieldInit>, Span),
    StoreDelete(Symbol, StoreFilter, Span),
    StoreDestroy(Symbol, StoreFilter, Span),
    StoreSet(Symbol, Vec<(Symbol, Expr)>, StoreFilter, Span),
    StoreRestore(Symbol, StoreFilter, Span),
    StoreSave(Symbol, Span),
    Transaction(Block, Span),
    ChannelClose(Expr, Span),
    Stop(Expr, Span),
    SimFor(For, Span),
    SimBlock(Block, Span),
    UseLocal(UseDecl),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Bind(b) => b.span,
            Stmt::TupleBind(_, _, s) => *s,
            Stmt::Assign(_, _, s) => *s,
            Stmt::Expr(e) => e.span(),
            Stmt::If(i) => i.span,
            Stmt::While(w) => w.span,
            Stmt::For(f) => f.span,
            Stmt::Loop(l) => l.span,
            Stmt::Ret(_, s) => *s,
            Stmt::Break(_, s) => *s,
            Stmt::Continue(s) => *s,
            Stmt::Nop(s) => *s,
            Stmt::Match(m) => m.span,
            Stmt::Asm(a) => a.span,
            Stmt::ErrReturn(_, s) => *s,
            Stmt::Defer(_, s) => *s,
            Stmt::StoreInsert(_, _, s) => *s,
            Stmt::StoreDelete(_, _, s) => *s,
            Stmt::StoreDestroy(_, _, s) => *s,
            Stmt::StoreSet(_, _, _, s) => *s,
            Stmt::StoreRestore(_, _, s) => *s,
            Stmt::StoreSave(_, s) => *s,
            Stmt::Transaction(_, s) => *s,
            Stmt::ChannelClose(_, s) => *s,
            Stmt::Stop(_, s) => *s,
            Stmt::SimFor(_, s) => *s,
            Stmt::SimBlock(_, s) => *s,
            Stmt::UseLocal(_) => Span::dummy(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Pat {
    Wild(Span),
    Ident(Symbol, Span),
    Lit(Expr),
    Ctor(Symbol, Vec<Pat>, Span),
    Or(Vec<Pat>, Span),
    Range(Expr, Expr, Span),
    Tuple(Vec<Pat>, Span),
    Array(Vec<Pat>, Span),
}

impl Pat {
    pub fn span(&self) -> Span {
        match self {
            Pat::Wild(s)
            | Pat::Ident(_, s)
            | Pat::Ctor(_, _, s)
            | Pat::Or(_, s)
            | Pat::Range(_, _, s)
            | Pat::Tuple(_, s)
            | Pat::Array(_, s) => *s,
            Pat::Lit(e) => e.span(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    None(Span),
    Void(Span),
    Int(i64, Span),
    Float(f64, Span),
    Str(String, Span),
    Bool(bool, Span),
    Ident(Symbol, Span),
    BinOp(Box<Expr>, BinOp, Box<Expr>, Span),
    UnaryOp(UnaryOp, Box<Expr>, Span),
    Call(Box<Expr>, Vec<Expr>, Span),
    Method(Box<Expr>, Symbol, Vec<Expr>, Span),
    Field(Box<Expr>, Symbol, Span),
    Index(Box<Expr>, Box<Expr>, Span),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>, Span),
    As(Box<Expr>, Type, Span),
    Array(Vec<Expr>, Span),
    Tuple(Vec<Expr>, Span),
    Struct(Symbol, Vec<FieldInit>, Span),
    IfExpr(Box<If>),
    Pipe(Box<Expr>, Box<Expr>, Vec<Expr>, Span),
    Block(Block, Span),
    Lambda(Vec<Param>, Option<Type>, Block, Span),
    Placeholder(Span),
    IndexPlaceholder(Span),
    Ref(Box<Expr>, Span),
    Deref(Box<Expr>, Span),
    Embed(String, Span),
    ListComp(
        Box<Expr>,
        String,
        Box<Expr>,
        Option<Box<Expr>>,
        Option<Box<Expr>>,
        Span,
    ),
    Syscall(Vec<Expr>, Span),
    Query(Box<Expr>, Vec<QueryClause>, Span),
    StoreQuery(Symbol, Box<StoreFilter>, Span),
    StoreCount(Symbol, Option<Box<StoreFilter>>, Span),
    StoreAll(Symbol, Span),
    StoreGet(Symbol, Box<Expr>, Span),
    StoreFirst(Symbol, Box<StoreFilter>, Span),
    StoreExists(Symbol, Box<StoreFilter>, Span),
    StoreDistinct(Symbol, Symbol, Span),
    Spawn(Symbol, Vec<(Symbol, Expr)>, Span),
    Send(Box<Expr>, Symbol, Vec<Expr>, Span),
    Receive(Vec<ReceiveArm>, Span),
    Yield(Box<Expr>, Span),
    DispatchBlock(Symbol, Block, Span),
    ChannelCreate(Option<Type>, Box<Expr>, Span),
    ChannelSend(Box<Expr>, Box<Expr>, Span),
    ChannelRecv(Box<Expr>, Span),
    Select(Vec<SelectArm>, Option<Block>, Span),
    Unreachable(Span),
    AsFormat(Box<Expr>, Symbol, Span),
    StrictCast(Box<Expr>, Type, Span),
    Slice(Box<Expr>, Box<Expr>, Box<Expr>, Span),
    NamedArg(Symbol, Box<Expr>, Span),
    Spread(Box<Expr>, Span),
    NDArray(Vec<Expr>, Span),
    SIMDLit(Type, usize, Vec<Expr>, Span),
    Grad(Box<Expr>, Span),
    Einsum(String, Vec<Expr>, Span),
    Builder(Symbol, Vec<BuilderField>, Span),
    Deque(Vec<Expr>, Span),
    OfCall(Box<Expr>, Box<Expr>, Span),
    QualifiedIdent(Symbol, Symbol, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::None(s)
            | Self::Void(s)
            | Self::Int(_, s)
            | Self::Float(_, s)
            | Self::Str(_, s)
            | Self::Bool(_, s)
            | Self::Ident(_, s)
            | Self::BinOp(_, _, _, s)
            | Self::UnaryOp(_, _, s)
            | Self::Call(_, _, s)
            | Self::Method(_, _, _, s)
            | Self::Field(_, _, s)
            | Self::Index(_, _, s)
            | Self::Ternary(_, _, _, s)
            | Self::As(_, _, s)
            | Self::Array(_, s)
            | Self::Tuple(_, s)
            | Self::Struct(_, _, s)
            | Self::Pipe(_, _, _, s)
            | Self::Block(_, s)
            | Self::Lambda(_, _, _, s)
            | Self::Placeholder(s)
            | Self::IndexPlaceholder(s)
            | Self::Ref(_, s)
            | Self::Deref(_, s)
            | Self::Embed(_, s)
            | Self::ListComp(_, _, _, _, _, s)
            | Self::Syscall(_, s)
            | Self::Query(_, _, s)
            | Self::StoreQuery(_, _, s)
            | Self::StoreCount(_, _, s)
            | Self::StoreAll(_, s)
            | Self::StoreGet(_, _, s)
            | Self::StoreFirst(_, _, s)
            | Self::StoreExists(_, _, s)
            | Self::StoreDistinct(_, _, s)
            | Self::Spawn(_, _, s)
            | Self::Send(_, _, _, s)
            | Self::Receive(_, s)
            | Self::Yield(_, s)
            | Self::DispatchBlock(_, _, s)
            | Self::ChannelCreate(_, _, s)
            | Self::ChannelSend(_, _, s)
            | Self::ChannelRecv(_, s)
            | Self::Select(_, _, s)
            | Self::Unreachable(s)
            | Self::AsFormat(_, _, s)
            | Self::StrictCast(_, _, s)
            | Self::Slice(_, _, _, s)
            | Self::NamedArg(_, _, s)
            | Self::Spread(_, s)
            | Self::NDArray(_, s)
            | Self::SIMDLit(_, _, _, s) => *s,
            Self::IfExpr(i) => i.span,
            Self::Grad(_, s)
            | Self::Einsum(_, _, s)
            | Self::Builder(_, _, s)
            | Self::Deque(_, s)
            | Self::OfCall(_, _, s)
            | Self::QualifiedIdent(_, _, s) => *s,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub struct Fn {
    pub name: Symbol,
    pub type_params: Vec<Symbol>,
    pub type_bounds: Vec<(Symbol, Vec<Symbol>)>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    /// Error types declared in the signature: `returns T ! E1 ! E2`.
    /// Empty when the user omits them; the typer infers from the body.
    pub error_types: Vec<Type>,
    pub body: Block,
    pub is_generator: bool,
    pub attrs: FnAttrs,
    pub span: Span,
}

/// Function attributes from `@inline`, `@noinline`, `@cold`, `@hot` annotations.
#[derive(Debug, Clone, Default)]
pub struct FnAttrs {
    pub inline: bool,
    pub noinline: bool,
    pub cold: bool,
    pub hot: bool,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: Symbol,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
    pub literal: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutAttrs {
    pub packed: bool,
    pub strict: bool,
    pub align: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: Symbol,
    pub type_params: Vec<Symbol>,
    pub fields: Vec<Field>,
    pub methods: Vec<Fn>,
    pub layout: LayoutAttrs,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: Symbol,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: Symbol,
    pub type_params: Vec<Symbol>,
    pub variants: Vec<Variant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: Symbol,
    pub fields: Vec<VField>,
    pub discriminant: Option<i64>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VField {
    pub name: Option<Symbol>,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Bind {
    pub name: Symbol,
    pub value: Expr,
    pub ty: Option<Type>,
    pub atomic: bool,
    pub span: Span,
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
    pub label: Option<Symbol>,
    pub bind: Symbol,
    pub bind2: Option<Symbol>,
    pub iter: Expr,
    pub end: Option<Expr>,
    pub step: Option<Expr>,
    pub body: Block,
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
pub struct ExternFn {
    pub name: Symbol,
    pub params: Vec<(Symbol, Type)>,
    pub ret: Type,
    pub variadic: bool,
    pub span: Span,
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

#[derive(Debug, Clone)]
pub enum QueryClause {
    Where(Expr, Span),
    Limit(Expr, Span),
    Sort(Symbol, bool, Span),
    Take(Expr, Span),
    Skip(Expr, Span),
    Set(Symbol, Expr, Span),
    Delete(Span),
}

#[derive(Debug, Clone)]
pub struct UseDecl {
    pub path: Vec<Symbol>,
    pub imports: Option<Vec<Symbol>>,
    pub alias: Option<Symbol>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ErrDef {
    pub name: Symbol,
    pub variants: Vec<ErrVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ErrVariant {
    pub name: Symbol,
    pub fields: Vec<Type>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TestBlock {
    pub name: Symbol,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ActorDef {
    pub name: Symbol,
    pub fields: Vec<Field>,
    pub handlers: Vec<Handler>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Handler {
    pub name: Symbol,
    pub params: Vec<Param>,
    pub is_loop: bool,
    pub loop_sleep_ms: Option<Expr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ReceiveArm {
    pub handler: Symbol,
    pub bindings: Vec<Symbol>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct SelectArm {
    pub is_send: bool,
    pub chan: Expr,
    pub value: Option<Expr>,
    pub binding: Option<Symbol>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreDecorator {
    Simple,
    Mem,
    Transient,
    Versioned,
    Vector(u64),
    Graph,
    TimeSeries(Symbol),
    Kv,
    BeforeInsert(Symbol),
    AfterInsert(Symbol),
    BeforeDelete(Symbol),
    AfterDelete(Symbol),
    Column,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldDecorator {
    Index,
    Unique,
    Sorted,
    Transient,
    Increment,
    Required,
    Versioned,
    Default(String),
    Cascade,
    Lazy,
    Bloom,
    Search,
}

#[derive(Debug, Clone)]
pub struct StoreField {
    pub name: Symbol,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
    pub decorators: Vec<FieldDecorator>,
    pub is_relation: bool,
    pub is_has_many: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StoreDef {
    pub name: Symbol,
    pub decorators: Vec<StoreDecorator>,
    pub fields: Vec<StoreField>,
    pub methods: Vec<Fn>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct MigrationDef {
    pub name: Symbol,
    pub version: i64,
    pub up: Vec<AlterOp>,
    pub down: Vec<AlterOp>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct AlterOp {
    pub store_name: Symbol,
    pub actions: Vec<AlterAction>,
}

#[derive(Debug, Clone)]
pub enum AlterAction {
    Add {
        name: String,
        ty: Type,
        default: Option<Expr>,
    },
    Drop {
        name: String,
    },
    Rename {
        from: String,
        to: String,
    },
}

#[derive(Debug, Clone)]
pub struct ViewDef {
    pub name: Symbol,
    pub source: Symbol,
    pub clauses: Vec<QueryClause>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TraitDef {
    pub name: Symbol,
    pub type_params: Vec<Symbol>,
    pub assoc_types: Vec<Symbol>,
    pub methods: Vec<TraitMethod>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TraitMethod {
    pub name: Symbol,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub default_body: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub trait_name: Option<Symbol>,
    pub trait_type_args: Vec<Type>,
    pub type_name: Symbol,
    pub assoc_type_bindings: Vec<(Symbol, Type)>,
    pub methods: Vec<Fn>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorStrategy {
    OneForOne,
    OneForAll,
    RestForOne,
}

#[derive(Debug, Clone)]
pub struct SupervisorDef {
    pub name: Symbol,
    pub strategy: SupervisorStrategy,
    pub children: Vec<Symbol>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct BuilderField {
    pub name: Symbol,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StoreFilter {
    pub field: Symbol,
    pub op: BinOp,
    pub value: Expr,
    pub span: Span,
    pub extra: Vec<(LogicalOp, StoreFilterCond)>,
}

#[derive(Debug, Clone)]
pub struct StoreFilterCond {
    pub field: Symbol,
    pub op: BinOp,
    pub value: Expr,
}
