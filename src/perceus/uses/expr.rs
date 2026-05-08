//! Sub-pass of perceus/uses split.

use std::collections::HashMap;

use crate::hir::*;

use super::super::{PerceusPass, UseInfo};

impl PerceusPass {
    pub(in crate::perceus) fn count_uses_pat(
        &mut self,
        pat: &Pat,
        uses: &mut HashMap<DefId, UseInfo>,
    ) {
        match pat {
            Pat::Wild(_) => {}
            Pat::Bind(def_id, _, ty, _) => {
                uses.insert(*def_id, UseInfo::new(ty.clone(), ty.default_ownership()));
                self.hints.stats.total_bindings_analyzed += 1;
            }
            Pat::Lit(e) => {
                self.count_uses_expr(e, uses);
            }
            Pat::Ctor(_, _, sub_pats, _) => {
                for sp in sub_pats {
                    self.count_uses_pat(sp, uses);
                }
            }
            Pat::Or(alts, _) => {
                for alt in alts {
                    self.count_uses_pat(alt, uses);
                }
            }
            Pat::Range(lo, hi, _) => {
                self.count_uses_expr(lo, uses);
                self.count_uses_expr(hi, uses);
            }
            Pat::Tuple(pats, _) | Pat::Array(pats, _) => {
                for p in pats {
                    self.count_uses_pat(p, uses);
                }
            }
        }
    }

    pub(in crate::perceus) fn count_uses_expr(
        &mut self,
        expr: &Expr,
        uses: &mut HashMap<DefId, UseInfo>,
    ) {
        match &expr.kind {
            ExprKind::Var(def_id, _) => {
                if let Some(info) = uses.get_mut(def_id) {
                    info.use_count += 1;
                    info.last_use_span = Some(expr.span);
                }
            }
            ExprKind::FnRef(_, _) | ExprKind::VariantRef(_, _, _) => {}
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Bool(_)
            | ExprKind::None
            | ExprKind::Void => {}

            ExprKind::BinOp(l, _, r) => {
                self.count_uses_expr(l, uses);
                self.count_uses_expr(r, uses);
            }
            ExprKind::UnaryOp(_, e) => {
                self.count_uses_expr(e, uses);
            }
            ExprKind::Call(_, _, args) => {
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::IndirectCall(callee, args) => {
                self.count_uses_expr(callee, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::Builtin(builtin, args) => {
                let escapes = !matches!(
                    builtin,
                    BuiltinFn::Log
                        | BuiltinFn::Print
                        | BuiltinFn::ToString
                        | BuiltinFn::Popcount
                        | BuiltinFn::Clz
                        | BuiltinFn::Ctz
                        | BuiltinFn::RotateLeft
                        | BuiltinFn::RotateRight
                        | BuiltinFn::Bswap
                        | BuiltinFn::WrappingAdd
                        | BuiltinFn::WrappingSub
                        | BuiltinFn::WrappingMul
                        | BuiltinFn::SaturatingAdd
                        | BuiltinFn::SaturatingSub
                        | BuiltinFn::SaturatingMul
                        | BuiltinFn::CheckedAdd
                        | BuiltinFn::CheckedSub
                        | BuiltinFn::CheckedMul
                        | BuiltinFn::SignalRaise
                        | BuiltinFn::SignalIgnore
                        | BuiltinFn::SignalDefault
                        | BuiltinFn::SignalKill
                );
                for a in args {
                    if escapes {
                        self.count_uses_expr_escaping(a, uses);
                    } else {
                        self.count_uses_expr(a, uses);
                    }
                }
            }
            ExprKind::Method(obj, _, _, args)
            | ExprKind::StringMethod(obj, _, args)
            | ExprKind::DeferredMethod(obj, _, args) => {
                self.count_uses_expr(obj, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::Field(obj, _, _) => {
                self.count_uses_expr(obj, uses);
            }
            ExprKind::Index(arr, idx) => {
                self.count_uses_expr(arr, uses);
                self.count_uses_expr(idx, uses);
            }
            ExprKind::Ternary(cond, then, els) => {
                self.count_uses_expr(cond, uses);
                self.count_uses_expr(then, uses);
                self.count_uses_expr(els, uses);
            }
            ExprKind::Coerce(inner, _) | ExprKind::Cast(inner, _) => {
                self.count_uses_expr(inner, uses);
            }
            ExprKind::Array(elems) | ExprKind::Tuple(elems) => {
                for e in elems {
                    self.count_uses_expr(e, uses);
                }
            }
            ExprKind::Struct(_, inits) | ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    self.count_uses_expr_escaping(&fi.value, uses);
                }
            }
            ExprKind::IfExpr(i) => {
                self.count_uses_expr(&i.cond, uses);
                self.count_uses_block(&i.then, uses);
                for (ec, eb) in &i.elifs {
                    self.count_uses_expr(ec, uses);
                    self.count_uses_block(eb, uses);
                }
                if let Some(els) = &i.els {
                    self.count_uses_block(els, uses);
                }
            }
            ExprKind::Pipe(first, _, _, rest) => {
                self.count_uses_expr_escaping(first, uses);
                for a in rest {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::Block(stmts) => {
                self.count_uses_block(stmts, uses);
            }
            ExprKind::Lambda(params, body) => {
                let mut lambda_uses: HashMap<DefId, UseInfo> = HashMap::new();
                for p in params {
                    lambda_uses.insert(p.def_id, UseInfo::new(p.ty.clone(), p.ownership));
                }
                let mut refs = Vec::new();
                self.collect_refs_block(body, &mut refs);
                for id in refs {
                    if let Some(info) = uses.get_mut(&id) {
                        info.escapes = true;
                        info.use_count += 1;
                    }
                }
            }
            ExprKind::Ref(inner) => {
                self.count_uses_expr(inner, uses);
                if let ExprKind::Var(def_id, _) = &inner.kind {
                    if let Some(info) = uses.get_mut(def_id) {
                        info.borrowed = true;
                    }
                }
            }
            ExprKind::Deref(inner) => {
                self.count_uses_expr(inner, uses);
            }
            ExprKind::ListComp(body, bind_id, _, iter, cond, map) => {
                self.count_uses_expr(iter, uses);
                uses.insert(*bind_id, UseInfo::new(body.ty.clone(), Ownership::Owned));
                self.count_uses_expr(body, uses);
                if let Some(c) = cond {
                    self.count_uses_expr(c, uses);
                }
                if let Some(m) = map {
                    self.count_uses_expr(m, uses);
                }
            }
            ExprKind::Syscall(args) => {
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::Spawn(_) => {}
            ExprKind::Unreachable => {}
            ExprKind::StrictCast(inner, _)
            | ExprKind::AsFormat(inner, _)
            | ExprKind::AtomicLoad(inner) => {
                self.count_uses_expr(inner, uses);
            }
            ExprKind::AtomicStore(a, b) | ExprKind::AtomicAdd(a, b) | ExprKind::AtomicSub(a, b) => {
                self.count_uses_expr(a, uses);
                self.count_uses_expr(b, uses);
            }
            ExprKind::AtomicCas(p, e, n) => {
                self.count_uses_expr(p, uses);
                self.count_uses_expr(e, uses);
                self.count_uses_expr(n, uses);
            }
            ExprKind::Slice(obj, start, end) => {
                self.count_uses_expr(obj, uses);
                self.count_uses_expr(start, uses);
                self.count_uses_expr(end, uses);
            }
            ExprKind::Send(target, _, _, _, args) => {
                self.count_uses_expr(target, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::StoreQuery(_, _)
            | ExprKind::StoreCount(_)
            | ExprKind::StoreAll(_)
            | ExprKind::ViewCount(_, _)
            | ExprKind::ViewAll(_, _)
            | ExprKind::StoreDistinct(_, _)
            | ExprKind::StoreSum(_, _)
            | ExprKind::StoreAvg(_, _)
            | ExprKind::StoreMin(_, _)
            | ExprKind::StoreMax(_, _)
            | ExprKind::StoreVersionCount(_, _)
            | ExprKind::StoreHistory(_, _)
            | ExprKind::StoreAtVersion(_, _, _) => {}
            ExprKind::StoreGet(_, key) => {
                self.count_uses_expr(key, uses);
            }
            ExprKind::StoreFirst(_, _) | ExprKind::StoreExists(_, _) => {}
            ExprKind::CoroutineCreate(_, body) => {
                self.count_uses_block(body, uses);
            }
            ExprKind::CoroutineNext(inner)
            | ExprKind::Yield(inner)
            | ExprKind::DynCoerce(inner, _, _) => {
                self.count_uses_expr(inner, uses);
            }
            ExprKind::DynDispatch(obj, _, _, args) => {
                self.count_uses_expr(obj, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::VecNew(args) => {
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::MapNew | ExprKind::SetNew | ExprKind::PQNew | ExprKind::NDArrayNew(_) => {}
            ExprKind::SIMDNew(elems) => {
                for e in elems {
                    self.count_uses_expr(e, uses);
                }
            }
            ExprKind::IterNext(_, _, _) => {}
            ExprKind::VecMethod(obj, _, args)
            | ExprKind::MapMethod(obj, _, args)
            | ExprKind::SetMethod(obj, _, args)
            | ExprKind::PQMethod(obj, _, args) => {
                self.count_uses_expr(obj, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::ChannelCreate(_, cap) => {
                self.count_uses_expr(cap, uses);
            }
            ExprKind::ChannelSend(ch, val) => {
                self.count_uses_expr(ch, uses);
                self.count_uses_expr_escaping(val, uses);
            }
            ExprKind::ChannelRecv(ch) => {
                self.count_uses_expr(ch, uses);
            }
            ExprKind::Select(arms, default_body) => {
                for arm in arms {
                    self.count_uses_expr(&arm.chan, uses);
                    if let Some(ref v) = arm.value {
                        self.count_uses_expr_escaping(v, uses);
                    }
                    if let Some(_bind_name) = &arm.binding {
                        if let Some(bind_id) = arm.bind_id {
                            uses.insert(
                                bind_id,
                                UseInfo::new(arm.elem_ty.clone(), Ownership::Owned),
                            );
                        }
                    }
                    self.count_uses_block(&arm.body, uses);
                }
                if let Some(body) = default_body {
                    self.count_uses_block(body, uses);
                }
            }
            ExprKind::DequeNew => {}
            ExprKind::DequeMethod(obj, _, args) => {
                self.count_uses_expr(obj, uses);
                for a in args {
                    self.count_uses_expr_escaping(a, uses);
                }
            }
            ExprKind::Grad(e)
            | ExprKind::CowWrap(e)
            | ExprKind::CowClone(e)
            | ExprKind::GeneratorNext(e)
            | ExprKind::EnumUnwrap(e, _, _)
            | ExprKind::EnumIs(e, _) => {
                self.count_uses_expr(e, uses);
            }
            ExprKind::Einsum(_, args) => {
                for a in args {
                    self.count_uses_expr(a, uses);
                }
            }
            ExprKind::Builder(_, fields) => {
                for (_, v) in fields {
                    self.count_uses_expr_escaping(v, uses);
                }
            }
            ExprKind::GeneratorCreate(_, _, stmts) => {
                self.count_uses_block(stmts, uses);
            }
            ExprKind::KvGet(_, e) | ExprKind::KvHas(_, e) | ExprKind::KvDel(_, e) => {
                self.count_uses_expr(e, uses)
            }
            ExprKind::KvSet(_, k, v) | ExprKind::KvIncr(_, k, v) => {
                self.count_uses_expr(k, uses);
                self.count_uses_expr(v, uses);
            }
            ExprKind::KvCount(_) | ExprKind::TsLatest(_) => {}
            ExprKind::VecNearest(_, v, k) => {
                self.count_uses_expr(v, uses);
                self.count_uses_expr(k, uses);
            }
            ExprKind::VecInsert(_, v) => self.count_uses_expr(v, uses),
            ExprKind::VecCount(_) | ExprKind::FtsCount(_, _) => {}
            ExprKind::BloomTest(_, _, v) => self.count_uses_expr(v, uses),
            ExprKind::FtsSearch(_, _, v) => self.count_uses_expr(v, uses),
            ExprKind::GraphFrom(_, e) | ExprKind::GraphTo(_, e) => self.count_uses_expr(e, uses),
            ExprKind::GlobalLoad(_) => {}
        }
    }

    pub(in crate::perceus) fn count_uses_expr_escaping(
        &mut self,
        expr: &Expr,
        uses: &mut HashMap<DefId, UseInfo>,
    ) {
        if let ExprKind::Var(def_id, _) = &expr.kind {
            if let Some(info) = uses.get_mut(def_id) {
                info.use_count += 1;
                info.escapes = true;
                info.last_use_span = Some(expr.span);
            }
        } else {
            self.count_uses_expr(expr, uses);
        }
    }
}
