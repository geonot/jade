// Auto-split from lower.rs.
#![allow(unused_imports, unused_variables)]
use crate::intern::Symbol;
use super::super::*;
use crate::ast::{self, Span};
use crate::hir::{self, ExprKind, Pat};
use crate::types::Type;
use std::collections::{HashMap, HashSet};
use super::Lowerer;

impl Lowerer {
    pub(super) fn lower_expr_p1(&mut self, expr: &hir::Expr) -> Option<ValueId> {
        let span = expr.span;
        let ty = expr.ty.clone();
        Some(match &expr.kind {
            ExprKind::VariantRef(enum_name, variant_name, tag) => self.emit(
                InstKind::VariantInit(*enum_name, *variant_name, *tag, vec![]),
                ty,
                span,
            ),

            ExprKind::Block(stmts) => self.lower_block_expr(stmts),

            ExprKind::Lambda(params, body) => {
                // Lower the lambda body as a separate MIR function.
                // Captured variables become leading parameters; declared params follow.
                let lambda_name = format!("lambda.{}", self.func.next_value);

                // Collect captured variable (name, ValueId, Type) triples.
                let param_names: std::collections::HashSet<Symbol> =
                    params.iter().map(|p| p.name).collect();
                let mut refs = std::collections::HashSet::new();
                Self::collect_expr_var_refs_block(body, &mut refs);
                let mut capture_info: Vec<(Symbol, ValueId, Type)> = Vec::new();
                for name in &refs {
                    if !param_names.contains(name) {
                        if let Some(&val) = self.var_map.get(name) {
                            let cap_ty = self.value_type(val);
                            capture_info.push((*name, val, cap_ty));
                        }
                    }
                }
                let capture_vals: Vec<ValueId> = capture_info.iter().map(|(_, v, _)| *v).collect();

                // Determine return type from the closure's type annotation.
                let ret_ty = if let Type::Fn(_, r) = &ty {
                    *r.clone()
                } else {
                    Type::I64
                };

                // Create a new Lowerer for the lambda function.
                let mut lambda_lowerer = Lowerer::new(&lambda_name, crate::hir::DefId(0), span);
                lambda_lowerer.func.ret_ty = ret_ty;

                // Add capture parameters first.
                for (cap_name, _, cap_ty) in &capture_info {
                    let val = lambda_lowerer.new_value();
                    lambda_lowerer.func.params.push(Param {
                        value: val,
                        name: *cap_name,
                        ty: cap_ty.clone(),
                    });
                    lambda_lowerer.var_map.insert(*cap_name, val);
                }

                // Add declared parameters.
                for p in params {
                    let val = lambda_lowerer.new_value();
                    lambda_lowerer.func.params.push(Param {
                        value: val,
                        name: p.name.clone(),
                        ty: p.ty.clone(),
                    });
                    lambda_lowerer.var_map.insert(p.name.clone(), val);
                }

                // Lower the lambda body.
                let mut last = lambda_lowerer.emit(InstKind::Void, Type::Void, span);
                for stmt in body {
                    last = lambda_lowerer.lower_stmt(stmt);
                }
                if !lambda_lowerer.current_block_has_terminator() {
                    lambda_lowerer.set_terminator(Terminator::Return(Some(last)));
                }

                // Collect the lambda function and any nested lambdas.
                self.lambda_fns.push(lambda_lowerer.func);
                self.lambda_fns.append(&mut lambda_lowerer.lambda_fns);

                self.emit(InstKind::ClosureCreate(Symbol::intern(&lambda_name), capture_vals), ty, span)
            }

            ExprKind::Coerce(inner, _) => self.lower_expr(inner),

            ExprKind::Pipe(inner, _def_id, name, extra_args) => {
                let mut args = vec![self.lower_expr(inner)];
                args.extend(extra_args.iter().map(|a| self.lower_expr(a)));
                self.emit(InstKind::Call(*name, args), ty, span)
            }

            // Collection methods — all follow the same pattern
            ExprKind::StringMethod(obj, name, args)
            | ExprKind::DeferredMethod(obj, name, args)
            | ExprKind::VecMethod(obj, name, args)
            | ExprKind::MapMethod(obj, name, args)
            | ExprKind::SetMethod(obj, name, args)
            | ExprKind::PQMethod(obj, name, args)
            | ExprKind::DequeMethod(obj, name, args) => {
                let obj_val = self.lower_expr(obj);
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::MethodCall(obj_val, *name, vals), ty, span)
            }

            ExprKind::VecNew(elems) | ExprKind::NDArrayNew(elems) | ExprKind::SIMDNew(elems) => {
                let vals: Vec<ValueId> = elems.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(InstKind::VecNew(vals), ty, span)
            }

            ExprKind::MapNew => self.emit(InstKind::MapInit, ty, span),

            ExprKind::SetNew => self.emit(InstKind::SetInit, ty, span),
            ExprKind::PQNew => self.emit(InstKind::PQInit, ty, span),
            ExprKind::DequeNew => self.emit(InstKind::DequeInit, ty, span),

            ExprKind::ListComp(body_expr, _def_id, bind, iter, end, cond) => {
                // Desugar: vec = VecNew(); for bind in iter..end { if cond { VecPush(vec, body) } }
                let vec_val = self.emit(InstKind::VecNew(vec![]), ty.clone(), span);
                let iter_val = self.lower_expr(iter);

                let cond_bb = self.new_block("listcomp.cond");
                let body_bb = self.new_block("listcomp.body");
                let inc_bb = self.new_block("listcomp.inc");
                let exit_bb = self.new_block("listcomp.exit");

                // Create loop index using Store/Load.
                let init_val = if end.is_some() {
                    iter_val
                } else {
                    self.emit(InstKind::IntConst(0), Type::I64, span)
                };
                let one = self.emit(InstKind::IntConst(1), Type::I64, span);
                let end_val = if let Some(e) = end {
                    self.lower_expr(e)
                } else {
                    self.emit(InstKind::VecLen(iter_val), Type::I64, span)
                };
                let idx_name = Symbol::intern(&format!("__listcomp_idx_{bind}"));
                self.emit_void_typed(InstKind::Store(idx_name, init_val), Type::I64, span);

                self.set_terminator(Terminator::Goto(cond_bb));
                self.switch_to(cond_bb);
                let idx = self.emit(InstKind::Load(idx_name), Type::I64, span);
                let cmp = self.emit(
                    InstKind::Cmp(CmpOp::Lt, idx, end_val, Type::I64),
                    Type::Bool,
                    span,
                );
                self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                self.switch_to(body_bb);
                // Bind the loop variable — for collection iteration (no end),
                // bind the element at the current index, not the index itself.
                if end.is_none() {
                    let elem = self.emit(InstKind::Index(iter_val, idx), ty.clone(), span);
                    self.var_map.insert(Symbol::intern(bind), elem);
                } else {
                    self.var_map.insert(Symbol::intern(bind), idx);
                }
                let elem_val = self.lower_expr(body_expr);
                if let Some(c) = cond {
                    let filter_bb = self.new_block("listcomp.filter");
                    let push_bb = self.new_block("listcomp.push");
                    let cond_val = self.lower_expr(c);
                    self.set_terminator(Terminator::Branch(cond_val, push_bb, filter_bb));

                    self.switch_to(push_bb);
                    self.emit_void(InstKind::VecPush(vec_val, elem_val), span);
                    self.set_terminator(Terminator::Goto(inc_bb));

                    self.switch_to(filter_bb);
                    self.set_terminator(Terminator::Goto(inc_bb));
                } else {
                    self.emit_void(InstKind::VecPush(vec_val, elem_val), span);
                    self.set_terminator(Terminator::Goto(inc_bb));
                }

                // Increment index.
                self.switch_to(inc_bb);
                let cur_idx = self.emit(InstKind::Load(idx_name), Type::I64, span);
                let next_idx =
                    self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, span);
                self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, span);
                self.set_terminator(Terminator::Goto(cond_bb));

                self.switch_to(exit_bb);
                vec_val
            }

            // Concurrency primitives — lower as dedicated MIR instructions
            ExprKind::Spawn(name) => {
                self.emit(InstKind::SpawnActor(*name, vec![]), ty, span)
            }
            ExprKind::Send(target, _type_name, handler, _tag, args) => {
                let mut all = vec![self.lower_expr(target)];
                all.extend(args.iter().map(|a| self.lower_expr(a)));
                self.emit(InstKind::Call(Symbol::intern(&format!("__send_{handler}")), all), ty, span)
            }
            ExprKind::ChannelCreate(elem_ty, cap) => {
                let cap_val = self.lower_expr(cap);
                self.emit(
                    InstKind::ChanCreate(elem_ty.clone(), Some(cap_val)),
                    ty,
                    span,
                )
            }
            ExprKind::ChannelSend(chan, val) => {
                let ch = self.lower_expr(chan);
                let v = self.lower_expr(val);
                self.emit(InstKind::ChanSend(ch, v), ty, span)
            }
            ExprKind::ChannelRecv(chan) => {
                let c = self.lower_expr(chan);
                self.emit(InstKind::ChanRecv(c), ty, span)
            }

            ExprKind::Select(arms, default) => {
                // Demote variables assigned in any arm to memory (Store/Load)
                // so the merge point sees correct values.
                let mut assigned = HashSet::new();
                for arm in arms.iter() {
                    Self::collect_assigned_vars(&arm.body, &mut assigned);
                }
                if let Some(def_body) = default {
                    Self::collect_assigned_vars(def_body, &mut assigned);
                }
                let pre_existing: HashSet<Symbol> = assigned
                    .iter()
                    .filter(|n| self.var_map.contains_key(*n))
                    .cloned()
                    .collect();
                self.demote_vars_to_memory(&pre_existing, span);

                // Lower select as a SelectArm with all channel values
                let ch_vals: Vec<ValueId> =
                    arms.iter().map(|arm| self.lower_expr(&arm.chan)).collect();
                let has_default = default.is_some();
                let select_val = self.emit(
                    InstKind::SelectArm(ch_vals.clone(), has_default),
                    ty.clone(),
                    span,
                );
                // Lower bodies as a switch on the selected arm index
                if !arms.is_empty() {
                    let merge_bb = self.new_block("select.merge");
                    let mut cases: Vec<(i64, BlockId)> = Vec::new();
                    for (i, arm) in arms.iter().enumerate() {
                        let arm_bb = self.new_block(&format!("select.arm{i}"));
                        cases.push((i as i64, arm_bb));
                        self.switch_to(arm_bb);
                        if let Some(bind_name) = &arm.binding {
                            // Use __select_recv instead of ChanRecv — jade_select
                            // already received the data into the case data buffer.
                            let idx_val = self.emit(InstKind::IntConst(i as i64), Type::I64, span);
                            let recv_val = self.emit(
                                InstKind::Call(
                                    Symbol::intern("__select_recv"),
                                    vec![select_val, idx_val],
                                ),
                                arm.elem_ty.clone(),
                                span,
                            );
                            self.var_map.insert(bind_name.clone(), recv_val);
                        }
                        self.lower_block_stmts(&arm.body);
                        self.set_terminator(Terminator::Goto(merge_bb));
                    }
                    let default_bb = if let Some(def_body) = default {
                        let db = self.new_block("select.default");
                        self.switch_to(db);
                        self.lower_block_stmts(def_body);
                        self.set_terminator(Terminator::Goto(merge_bb));
                        db
                    } else {
                        merge_bb
                    };
                    // We need to go back and set the switch terminator
                    // The select_val block ended where we emitted SelectArm
                    // Find the block that contains the select inst
                    let select_block = self
                        .func
                        .blocks
                        .iter()
                        .find(|b| b.insts.iter().any(|i| i.dest == Some(select_val)))
                        .map(|b| b.id)
                        .unwrap_or(self.current_block);
                    self.func.block_mut(select_block).terminator =
                        Terminator::Switch(select_val, cases, default_bb);
                    self.switch_to(merge_bb);
                }
                // Return 0 from the merge block, not the jade_select result
                self.emit(InstKind::IntConst(0), Type::I64, span)
            }

            // Atomics — all lowered as intrinsic calls
            ExprKind::AtomicLoad(p) => {
                let v = self.lower_expr(p);
                self.emit(InstKind::Call("__atomic_load".into(), vec![v]), ty, span)
            }
            ExprKind::AtomicStore(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_store".into(), args), ty, span)
            }
            ExprKind::AtomicAdd(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_add".into(), args), ty, span)
            }
            ExprKind::AtomicSub(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_sub".into(), args), ty, span)
            }
            ExprKind::AtomicCas(p, expected, desired) => {
                let args = vec![
                    self.lower_expr(p),
                    self.lower_expr(expected),
                    self.lower_expr(desired),
                ];
                self.emit(InstKind::Call("__atomic_cas".into(), args), ty, span)
            }

            // Builtin functions — dedicated MIR instructions for optimizable ones
            ExprKind::Builtin(builtin, args) => {
                use crate::hir::BuiltinFn;
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                match builtin {
                    BuiltinFn::Log => {
                        let arg_ty = args.first().map(|a| a.ty.clone()).unwrap_or(Type::I64);
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        // NOTE: inst.ty carries the argument type (not Void) so
                        // codegen can determine the format specifier for printing.
                        self.emit(InstKind::Log(v), arg_ty, span)
                    }
                    BuiltinFn::Assert => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::Assert(v, "assertion failed".into()), ty, span)
                    }
                    BuiltinFn::RcAlloc => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::RcNew(v, ty.clone()), ty, span)
                    }
                    BuiltinFn::RcRetain => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::RcClone(v), ty, span)
                    }
                    BuiltinFn::RcRelease => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::RcDec(v), ty, span)
                    }
                    BuiltinFn::WeakUpgrade => {
                        let v = vals
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::WeakUpgrade(v), ty, span)
                    }
                    _ => {
                        let name = Symbol::intern(&format!("__builtin_{builtin:?}"));
                        self.emit(InstKind::Call(name, vals), ty, span)
                    }
                }
            }

            // Syscall — opaque call
            ExprKind::Syscall(args) => {
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call("__syscall".into(), vals), ty, span)
            }

            // Coroutines — opaque calls
            _ => return None,
        })
    }
}
