use super::*;
use crate::ast;
use crate::hir::{self, ExprKind};
use crate::types::Type;

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
mod ssa;
mod stmt;
mod store_expr;
mod store_stmt;

use ctx::Lowerer;

pub fn lower_program(prog: &hir::Program) -> Program {
    let mut functions = Vec::new();
    for f in &prog.fns {
        functions.extend(lower_function(f));
    }

    for td in &prog.types {
        for m in &td.methods {
            functions.extend(lower_function(m));
        }
    }

    for ti in &prog.trait_impls {
        for m in &ti.methods {
            functions.extend(lower_function(m));
        }
    }

    // Actor handlers lower to standalone MIR functions taking the actor state
    // pointer (+ message params). The codegen actor driver emits the mailbox/
    // dispatch loop and calls into these.
    for ad in &prog.actors {
        functions.extend(lower_actor_init(ad));
        if let Some(sleep_fn) = lower_actor_sleep(ad) {
            functions.extend(sleep_fn);
        }
        for h in &ad.handlers {
            functions.extend(lower_handler(ad, h));
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

    for p in &f.params {
        let val = lowerer.new_value();
        lowerer.func.params.push(Param {
            value: val,
            name: p.name.clone(),
            ty: p.ty.clone(),
            ownership: p.ownership,
        });
        lowerer.var_types.insert(p.name.clone(), p.ty.clone());
        // Seed Braun's per-block definition map so `read_var` at the entry
        // block resolves the parameter directly.
        let entry = lowerer.func.entry;
        lowerer
            .current_def
            .entry(entry)
            .or_default()
            .insert(p.name.clone(), val);
    }

    finish_body(
        &mut lowerer,
        &f.body,
        &f.ret,
        f.span,
        f.name.as_str() == "main",
    );

    let mut result = vec![lowerer.func];
    result.append(&mut lowerer.lambda_fns);
    result
}

/// Lower a function/handler body (a statement list) into the current
/// `Lowerer`, including tail-expression-as-return handling and terminator
/// synthesis for fall-through paths. Shared by `lower_function` and
/// `lower_handler`. The `Lowerer`'s params and (for handlers) `field_ctx`
/// must already be set up before calling.
fn finish_body(
    lowerer: &mut Lowerer,
    body: &[hir::Stmt],
    ret_ty: &Type,
    span: Span,
    is_main: bool,
) {
    let tail_idx: Option<usize> = body
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
    let mut last = lowerer.emit(InstKind::Void, Type::Void, span);
    for (idx, stmt) in body.iter().enumerate() {
        let v = if Some(idx) == tail_idx {
            if let hir::Stmt::Expr(e) = stmt {
                lowerer.lower_expr_owned(e)
            } else {
                lowerer.lower_stmt(stmt)
            }
        } else {
            lowerer.lower_stmt(stmt)
        };

        if !matches!(stmt, hir::Stmt::Drop(..)) {
            last = v;
        }
    }

    if matches!(
        lowerer.func.block(lowerer.current_block).terminator,
        Terminator::Unreachable
    ) {
        // If the trailing open block is actually unreachable (e.g. it's a
        // dead block created after an explicit `Ret`/`ErrReturn`), leave it
        // as `Unreachable` — synthesizing a return here would emit a value
        // of the wrong type and confuse downstream passes.
        let cur = lowerer.current_block;
        // Forward-BFS reachability from entry. A block can have non-empty
        // `preds` yet still be unreachable: e.g. the `after.ret` dead block
        // gets a Goto(merge_bb) terminator set unconditionally by the if-stmt
        // lowering even when the body ended with an explicit `return`.
        // Those dead blocks register as predecessors of `merge_bb` but have
        // no predecessors themselves, so they are not reachable from entry.
        let is_reachable = {
            let entry = lowerer.func.entry;
            let mut visited = std::collections::HashSet::new();
            let mut stack = vec![entry];
            while let Some(b) = stack.pop() {
                if !visited.insert(b) {
                    continue;
                }
                if b == cur {
                    break;
                }
                for s in lowerer.func.block(b).terminator.successors() {
                    if !visited.contains(&s) {
                        stack.push(s);
                    }
                }
            }
            visited.contains(&cur)
        };

        if !is_reachable {
            // Already Unreachable — nothing to do.
        } else {
            lowerer.lower_deferred_in_reverse();
            if matches!(ret_ty, Type::Void) {
                lowerer.set_terminator(Terminator::Return(None));
            } else {
                // Special case: `main` is declared `-> i32` (exit code) but
                // the user is not required to explicitly `return 0`. When
                // the body falls through without producing an i32,
                // synthesize one.
                let last_ty = lowerer
                    .func
                    .blocks
                    .iter()
                    .flat_map(|b| b.insts.iter())
                    .find(|i| i.dest == Some(last))
                    .map(|i| i.ty.clone())
                    .unwrap_or(Type::Void);
                let ret_val =
                    if is_main && matches!(ret_ty, Type::I32) && !matches!(last_ty, Type::I32) {
                        lowerer.emit(InstKind::IntConst(0), Type::I32, span)
                    } else {
                        last
                    };
                lowerer.set_terminator(Terminator::Return(Some(ret_val)));
            }
        }
    }
}

/// Lower a single actor handler into a standalone MIR `Function`. The handler
/// takes the actor state struct pointer as its first parameter (`__self_state`,
/// or an explicit `self` param when present) followed by the message
/// parameters. Field references in the body (bare `Var`s carrying field
/// `DefId`s, or `self.field` for the explicit-self form) are redirected to
/// load/store through the state struct via the `field_ctx`.
fn lower_handler(actor: &hir::ActorDef, handler: &hir::HandlerDef) -> Vec<Function> {
    let fn_name = actor_handler_fn_name(actor.name.clone(), handler);
    let mut lowerer = Lowerer::new(&fn_name, actor.def_id, handler.span);
    lowerer.func.ret_ty = Type::Void;

    let state_struct_name = Symbol::intern(&format!("{}_state", actor.name));
    let state_ptr_ty = Type::Ptr(Box::new(Type::Struct(state_struct_name.clone(), vec![])));

    // Detect the explicit-self form (`@handler self, ...`): the leading param
    // named `self` IS the state pointer rather than a message argument.
    let has_explicit_self = handler
        .params
        .first()
        .is_some_and(|p| p.name.as_str() == "self");

    // First parameter: the state pointer. Use the explicit `self` name when
    // present so `self.field` reads resolve to it; otherwise a synthetic name.
    let self_name = if has_explicit_self {
        handler.params[0].name.clone()
    } else {
        Symbol::intern("__self_state")
    };
    let self_val = lowerer.new_value();
    lowerer.func.params.push(Param {
        value: self_val,
        name: self_name.clone(),
        ty: state_ptr_ty.clone(),
        // Raw: the state pointer aliases the live actor mailbox; do not let
        // the ownership tagging attach `noalias`.
        ownership: hir::Ownership::Raw,
    });
    let entry = lowerer.func.entry;
    lowerer
        .var_types
        .insert(self_name.clone(), state_ptr_ty.clone());
    lowerer
        .current_def
        .entry(entry)
        .or_default()
        .insert(self_name, self_val);

    // Message parameters (skip the explicit `self`, which is the state ptr).
    let msg_params = if has_explicit_self {
        &handler.params[1..]
    } else {
        &handler.params[..]
    };
    for p in msg_params {
        let val = lowerer.new_value();
        lowerer.func.params.push(Param {
            value: val,
            name: p.name.clone(),
            ty: p.ty.clone(),
            ownership: p.ownership,
        });
        lowerer.var_types.insert(p.name.clone(), p.ty.clone());
        lowerer
            .current_def
            .entry(entry)
            .or_default()
            .insert(p.name.clone(), val);
    }

    // Field context: map each field's canonical DefId to (name, type) so bare
    // field references in the body redirect to the state struct.
    let mut map = std::collections::HashMap::new();
    for (f, &fid) in actor.fields.iter().zip(actor.field_def_ids.iter()) {
        map.insert(fid, (f.name.clone(), f.ty.clone()));
    }
    lowerer.field_ctx = Some(ctx::FieldCtx {
        self_state: self_val,
        state_ty: Type::Struct(state_struct_name, vec![]),
        map,
    });

    finish_body(
        &mut lowerer,
        &handler.body,
        &Type::Void,
        handler.span,
        false,
    );

    let mut result = vec![lowerer.func];
    result.append(&mut lowerer.lambda_fns);
    result
}

/// Build a `Lowerer` set up like an actor handler: a single `__self_state`
/// pointer parameter and a `field_ctx` mapping each field's canonical `DefId`
/// to its `(name, type)`, so bare field references in synthesized bodies
/// redirect to load/store through the actor state struct.
fn actor_state_lowerer(actor: &hir::ActorDef, fn_name: &str) -> Lowerer {
    let mut lowerer = Lowerer::new(fn_name, actor.def_id, actor.span);

    let state_struct_name = Symbol::intern(&format!("{}_state", actor.name));
    let state_ptr_ty = Type::Ptr(Box::new(Type::Struct(state_struct_name.clone(), vec![])));

    let self_name = Symbol::intern("__self_state");
    let self_val = lowerer.new_value();
    lowerer.func.params.push(Param {
        value: self_val,
        name: self_name.clone(),
        ty: state_ptr_ty,
        // Raw: the state pointer aliases the live actor mailbox; do not let the
        // ownership tagging attach `noalias`.
        ownership: hir::Ownership::Raw,
    });
    let entry = lowerer.func.entry;
    let self_ty = lowerer.func.params.last().unwrap().ty.clone();
    lowerer.var_types.insert(self_name.clone(), self_ty);
    lowerer
        .current_def
        .entry(entry)
        .or_default()
        .insert(self_name, self_val);

    let mut map = std::collections::HashMap::new();
    for (f, &fid) in actor.fields.iter().zip(actor.field_def_ids.iter()) {
        map.insert(fid, (f.name.clone(), f.ty.clone()));
    }
    lowerer.field_ctx = Some(ctx::FieldCtx {
        self_state: self_val,
        state_ty: Type::Struct(state_struct_name, vec![]),
        map,
    });

    lowerer
}

/// Lower an actor's field-default initializers into a standalone MIR function
/// `__actor_init_<name>(ptr state)`. The codegen actor factory calls this
/// (passing the actor state struct pointer) to populate field defaults before
/// applying user-supplied spawn overrides. Fields without a default that are
/// `Vec`/`Map` get an empty container; scalar fields without a default are left
/// zero-initialized by the factory's `memset` and so are skipped here.
fn lower_actor_init(actor: &hir::ActorDef) -> Vec<Function> {
    let fn_name = actor_init_fn_name(actor.name.clone());
    let mut lowerer = actor_state_lowerer(actor, &fn_name);
    lowerer.func.ret_ty = Type::Void;

    // Synthesize one assignment per field that needs initialization. The
    // assignment target is a bare `Var` carrying the field's canonical DefId,
    // which `field_ctx` redirects to a `FieldSet` on the state struct.
    let mut body: Vec<hir::Stmt> = Vec::new();
    for (f, &fid) in actor.fields.iter().zip(actor.field_def_ids.iter()) {
        let value = if let Some(def) = &f.default {
            def.clone()
        } else {
            match &f.ty {
                Type::Vec(_) => hir::Expr {
                    kind: ExprKind::VecNew(vec![]),
                    ty: f.ty.clone(),
                    span: f.span,
                },
                Type::Map(_, _) => hir::Expr {
                    kind: ExprKind::MapNew,
                    ty: f.ty.clone(),
                    span: f.span,
                },
                _ => continue,
            }
        };
        let target = hir::Expr {
            kind: ExprKind::Var(fid, f.name.clone()),
            ty: f.ty.clone(),
            span: f.span,
        };
        body.push(hir::Stmt::Assign(target, value, f.span));
    }

    finish_body(&mut lowerer, &body, &Type::Void, actor.span, false);

    let mut result = vec![lowerer.func];
    result.append(&mut lowerer.lambda_fns);
    result
}

/// Lower an actor loop handler's sleep-duration expression into a standalone
/// MIR function `__actor_sleep_<name>(ptr state) -> i64`. Returns `None` when
/// the actor has no `loop` handler or that handler declares no sleep interval.
/// The codegen actor loop calls this each iteration to obtain the millisecond
/// count, keeping the sleep/yield mechanics in hand-built LLVM.
fn lower_actor_sleep(actor: &hir::ActorDef) -> Option<Vec<Function>> {
    let loop_h = actor.handlers.iter().find(|h| h.is_loop)?;
    let sleep_expr = loop_h.loop_sleep_ms.as_ref()?;

    let fn_name = actor_sleep_fn_name(actor.name.clone());
    let mut lowerer = actor_state_lowerer(actor, &fn_name);
    lowerer.func.ret_ty = Type::I64;

    let body = vec![hir::Stmt::Expr(sleep_expr.clone())];
    finish_body(&mut lowerer, &body, &Type::I64, sleep_expr.span, false);

    let mut result = vec![lowerer.func];
    result.append(&mut lowerer.lambda_fns);
    Some(result)
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
