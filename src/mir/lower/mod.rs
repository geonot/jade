//! HIR → MIR lowering.
//!
//! Converts HIR functions into MIR basic blocks with explicit control flow.

use super::*;
use crate::ast;
use crate::hir::{self, ExprKind};
use crate::types::Type;

mod analysis;
mod closures;
mod collections;
mod concurrency;
mod control;
mod ctx;
mod effects;
mod expr;
mod expr_control;
mod intrinsics;
mod loops;
mod stmt;
mod store_expr;
mod store_stmt;

use ctx::Lowerer;

pub fn lower_program(prog: &hir::Program) -> Program {
    let mut functions = Vec::new();
    for f in &prog.fns {
        functions.extend(lower_function(f));
    }
    // Also lower type methods
    for td in &prog.types {
        for m in &td.methods {
            functions.extend(lower_function(m));
        }
    }
    // Also lower trait impl methods
    for ti in &prog.trait_impls {
        for m in &ti.methods {
            functions.extend(lower_function(m));
        }
    }
    let types = prog
        .types
        .iter()
        .map(|td| TypeDef {
            name: td.name.clone(),
            fields: td
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ty.clone()))
                .collect(),
        })
        .collect();
    let externs = prog
        .externs
        .iter()
        .map(|ef| ExternDecl {
            name: ef.name.clone(),
            params: ef.params.iter().map(|p| p.1.clone()).collect(),
            ret: ef.ret.clone(),
        })
        .collect();
    let globals = prog
        .globals
        .iter()
        .map(|g| GlobalDef {
            name: g.name.clone(),
            ty: g.ty.clone(),
        })
        .collect();
    Program {
        functions,
        types,
        externs,
        globals,
    }
}

fn lower_function(f: &hir::Fn) -> Vec<Function> {
    let mut lowerer = Lowerer::new(&f.name.as_str(), f.def_id, f.span);
    lowerer.func.ret_ty = f.ret.clone();
    lowerer.func.attrs = f.attrs.clone();

    // Create value IDs for parameters
    for p in &f.params {
        let val = lowerer.new_value();
        lowerer.func.params.push(Param {
            value: val,
            name: p.name.clone(),
            ty: p.ty.clone(),
        });
        lowerer.var_map.insert(p.name.clone(), val);
    }

    // Lower body. We pre-identify the index of the *implicit-return*
    // statement — the last non-Drop, non-jump stmt that produces the
    // function's return value. If that statement is a bare `Stmt::Expr(e)`
    // (i.e. a tail expression, not an explicit `return`), we lower its
    // expression via `lower_expr_owned` so that a heap-typed field/index
    // read at the tail position is auto-cloned. Without this, the SSA
    // return value aliases storage owned by a local that is about to be
    // scope-exit dropped, producing a use-after-free in the caller.
    let tail_idx: Option<usize> = f
        .body
        .iter()
        .enumerate()
        .rev()
        .find(|(_, s)| {
            !matches!(
                s,
                hir::Stmt::Drop(..)
                    | hir::Stmt::Ret(..)
                    | hir::Stmt::Break(..)
                    | hir::Stmt::Continue(..)
                    | hir::Stmt::ErrReturn(..)
            )
        })
        .map(|(i, _)| i);
    let mut last = lowerer.emit(InstKind::Void, Type::Void, f.span);
    for (idx, stmt) in f.body.iter().enumerate() {
        let v = if Some(idx) == tail_idx {
            if let hir::Stmt::Expr(e) = stmt {
                lowerer.lower_expr_owned(e)
            } else {
                lowerer.lower_stmt(stmt)
            }
        } else {
            lowerer.lower_stmt(stmt)
        };
        // Don't let Drop/void statements clobber the result value
        // for non-void functions (drops are inserted by perceus after
        // the last-expression that should be returned).
        if !matches!(stmt, hir::Stmt::Drop(..)) {
            last = v;
        }
    }

    // Add implicit return if not already terminated
    if matches!(
        lowerer.func.block(lowerer.current_block).terminator,
        Terminator::Unreachable
    ) {
        lowerer.lower_deferred_in_reverse();
        if matches!(f.ret, Type::Void) {
            lowerer.set_terminator(Terminator::Return(None));
        } else {
            lowerer.set_terminator(Terminator::Return(Some(last)));
        }
    }

    let mut result = vec![lowerer.func];
    result.append(&mut lowerer.lambda_fns);
    result
}

fn lower_binop(op: &ast::BinOp) -> BinOp {
    match op {
        ast::BinOp::Add => BinOp::Add,
        ast::BinOp::Sub => BinOp::Sub,
        ast::BinOp::Mul => BinOp::Mul,
        ast::BinOp::Div => BinOp::Div,
        ast::BinOp::Mod => BinOp::Mod,
        ast::BinOp::Exp => BinOp::Exp,
        ast::BinOp::BitAnd => BinOp::BitAnd,
        ast::BinOp::BitOr => BinOp::BitOr,
        ast::BinOp::BitXor => BinOp::BitXor,
        ast::BinOp::Shl => BinOp::Shl,
        ast::BinOp::Shr => BinOp::Shr,
        ast::BinOp::Ushr => BinOp::Ushr,
        ast::BinOp::And => BinOp::And,
        ast::BinOp::Or => BinOp::Or,
        // Comparisons handled separately in lower_expr; this path is unreachable.
        ast::BinOp::Eq
        | ast::BinOp::Ne
        | ast::BinOp::Lt
        | ast::BinOp::Gt
        | ast::BinOp::Le
        | ast::BinOp::Ge => {
            unreachable!("comparison ops should be handled by lower_expr, not lower_binop")
        }
    }
}

fn lower_unaryop(op: &ast::UnaryOp) -> UnaryOp {
    match op {
        ast::UnaryOp::Neg => UnaryOp::Neg,
        ast::UnaryOp::Not => UnaryOp::Not,
        ast::UnaryOp::BitNot => UnaryOp::BitNot,
    }
}

impl Lowerer {
    pub(super) fn lower_expr(&mut self, expr: &hir::Expr) -> ValueId {
        match &expr.kind {
            ExprKind::BinOp(_, ast::BinOp::And | ast::BinOp::Or, _)
            | ExprKind::Block(..)
            | ExprKind::IfExpr(..)
            | ExprKind::Ternary(..) => self.lower_expr_control(expr),
            ExprKind::Int(..)
            | ExprKind::Float(..)
            | ExprKind::Bool(..)
            | ExprKind::Str(..)
            | ExprKind::Void
            | ExprKind::None
            | ExprKind::Var(..)
            | ExprKind::BinOp(..)
            | ExprKind::UnaryOp(..)
            | ExprKind::Call(..)
            | ExprKind::IndirectCall(..)
            | ExprKind::Method(..)
            | ExprKind::Field(..)
            | ExprKind::Index(..)
            | ExprKind::Struct(..)
            | ExprKind::VariantCtor(..)
            | ExprKind::VariantRef(..)
            | ExprKind::Coerce(..)
            | ExprKind::Pipe(..)
            | ExprKind::Cast(..)
            | ExprKind::StrictCast(..)
            | ExprKind::Ref(..)
            | ExprKind::Deref(..)
            | ExprKind::Array(..)
            | ExprKind::Tuple(..)
            | ExprKind::Slice(..)
            | ExprKind::FnRef(..)
            | ExprKind::Builder(..)
            | ExprKind::AsFormat(..)
            | ExprKind::EnumUnwrap(..)
            | ExprKind::EnumIs(..)
            | ExprKind::GlobalLoad(..)
            | ExprKind::Unreachable => self.lower_expr_value(expr),
            ExprKind::Lambda(..) => self.lower_expr_closure(expr),
            ExprKind::StringMethod(..)
            | ExprKind::DeferredMethod(..)
            | ExprKind::VecMethod(..)
            | ExprKind::MapMethod(..)
            | ExprKind::VecNew(..)
            | ExprKind::MapNew
            | ExprKind::ListComp(..)
            | ExprKind::IterNext(..) => self.lower_expr_collections(expr),
            ExprKind::Spawn(..)
            | ExprKind::Send(..)
            | ExprKind::ChannelCreate(..)
            | ExprKind::ChannelSend(..)
            | ExprKind::ChannelRecv(..)
            | ExprKind::Select(..)
            | ExprKind::CoroutineCreate(..)
            | ExprKind::CoroutineNext(..)
            | ExprKind::Yield(..)
            | ExprKind::GeneratorCreate(..)
            | ExprKind::GeneratorNext(..) => self.lower_expr_concurrency(expr),
            ExprKind::StoreQuery(..)
            | ExprKind::StoreCount(..)
            | ExprKind::StoreAll(..)
            | ExprKind::ViewCount(..)
            | ExprKind::ViewAll(..)
            | ExprKind::StoreGet(..)
            | ExprKind::StoreFirst(..)
            | ExprKind::StoreExists(..)
            | ExprKind::StoreDistinct(..)
            | ExprKind::StoreSum(..)
            | ExprKind::StoreAvg(..)
            | ExprKind::StoreMin(..)
            | ExprKind::StoreMax(..)
            | ExprKind::StoreVersionCount(..)
            | ExprKind::StoreHistory(..)
            | ExprKind::StoreAtVersion(..)
            | ExprKind::KvGet(..)
            | ExprKind::KvHas(..)
            | ExprKind::KvCount(..)
            | ExprKind::KvSet(..)
            | ExprKind::KvDel(..)
            | ExprKind::KvIncr(..)
            | ExprKind::VecInsert(..)
            | ExprKind::VecCount(..)
            | ExprKind::BloomTest(..)
            | ExprKind::FtsSearch(..)
            | ExprKind::FtsCount(..)
            | ExprKind::VecNearest(..)
            | ExprKind::GraphFrom(..)
            | ExprKind::GraphTo(..)
            | ExprKind::TsLatest(..) => self.lower_expr_store(expr),
            ExprKind::AtomicLoad(..)
            | ExprKind::AtomicStore(..)
            | ExprKind::AtomicAdd(..)
            | ExprKind::AtomicSub(..)
            | ExprKind::AtomicCas(..)
            | ExprKind::Builtin(..)
            | ExprKind::Syscall(..)
            | ExprKind::Grad(..)
            | ExprKind::Einsum(..) => self.lower_expr_intrinsics(expr),
        }
    }

    /// Lower an expression at a *consuming* / escape boundary. When the
    /// expression is a heap-typed field or index read — i.e. the codegen
    /// would otherwise hand us a pointer-share into a parent's storage —
    /// emit an implicit deep `Clone` so the consumer gets an independent
    /// owned value. This is the P4 auto-copy at all binding /
    /// struct-init / call-arg / return / send / spawn-init sites.
    ///
    /// For everything else (function calls that already produce a fresh
    /// owned value, struct literals, primitive ops, etc.) this falls
    /// through to plain `lower_expr`.
    pub(super) fn lower_expr_owned(&mut self, expr: &hir::Expr) -> ValueId {
        let v = self.lower_expr(expr);
        if Self::needs_auto_clone(expr) {
            self.emit(
                InstKind::Clone(v, expr.ty.clone()),
                expr.ty.clone(),
                expr.span,
            )
        } else {
            v
        }
    }

    /// True iff the expression is a field- or index-read whose result is
    /// a heap-owning value that would otherwise alias the parent's
    /// storage. Callers use this from consuming positions.
    fn needs_auto_clone(expr: &hir::Expr) -> bool {
        if expr.ty.is_trivially_droppable() || !expr.ty.is_value_clonable() {
            return false;
        }
        matches!(expr.kind, ExprKind::Field(..) | ExprKind::Index(..))
    }

    pub(super) fn lower_stmt(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::Bind(..)
            | hir::Stmt::Assign(..)
            | hir::Stmt::Expr(..)
            | hir::Stmt::Drop(..)
            | hir::Stmt::TupleBind(..)
            | hir::Stmt::Defer(..) => self.lower_stmt_core(stmt),
            hir::Stmt::Nop(span) => self.emit(InstKind::Void, Type::Void, *span),
            hir::Stmt::If(..)
            | hir::Stmt::Match(..)
            | hir::Stmt::Ret(..)
            | hir::Stmt::ErrReturn(..)
            | hir::Stmt::Break(..)
            | hir::Stmt::Continue(..) => self.lower_stmt_control(stmt),
            hir::Stmt::While(..)
            | hir::Stmt::For(..)
            | hir::Stmt::Loop(..)
            | hir::Stmt::SimFor(..) => self.lower_stmt_loops(stmt),
            hir::Stmt::StoreInsert(..)
            | hir::Stmt::StoreDelete(..)
            | hir::Stmt::StoreDestroy(..)
            | hir::Stmt::StoreRestore(..)
            | hir::Stmt::StoreSave(..)
            | hir::Stmt::StoreSet(..)
            | hir::Stmt::Transaction(..) => self.lower_stmt_store(stmt),
            hir::Stmt::ChannelClose(..)
            | hir::Stmt::Stop(..)
            | hir::Stmt::Asm(..)
            | hir::Stmt::SimBlock(..)
            | hir::Stmt::UseLocal(..)
            | hir::Stmt::GlobalStore(..) => self.lower_stmt_effects(stmt),
        }
    }
}
