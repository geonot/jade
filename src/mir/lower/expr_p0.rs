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
    pub(super) fn lower_expr_p0(&mut self, expr: &hir::Expr) -> Option<ValueId> {
        let span = expr.span;
        let ty = expr.ty.clone();
        Some(match &expr.kind {
            ExprKind::Int(n) => self.emit(InstKind::IntConst(*n), ty, span),
            ExprKind::Float(f) => self.emit(InstKind::FloatConst(*f), ty, span),
            ExprKind::Bool(b) => self.emit(InstKind::BoolConst(*b), ty, span),
            ExprKind::Str(s) => self.emit(InstKind::StringConst(s.clone()), ty, span),
            ExprKind::Void => self.emit(InstKind::Void, Type::Void, span),
            ExprKind::None => self.emit(InstKind::IntConst(0), ty, span),

            ExprKind::Var(_, name) => {
                if let Some(&val) = self.var_map.get(name) {
                    val
                } else {
                    self.emit(InstKind::Load(name.clone()), ty, span)
                }
            }

            // HIR uses ast::BinOp which includes comparison operators
            ExprKind::BinOp(lhs, op, rhs) => {
                // Short-circuit And/Or: evaluate rhs only if needed.
                if *op == ast::BinOp::And {
                    let l = self.lower_expr(lhs);
                    let false_val = self.emit(InstKind::BoolConst(false), Type::Bool, span);
                    let rhs_bb = self.new_block("and.rhs");
                    let merge_bb = self.new_block("and.merge");
                    let cur_bb = self.current_block;
                    self.set_terminator(Terminator::Branch(l, rhs_bb, merge_bb));
                    self.switch_to(rhs_bb);
                    let r = self.lower_expr(rhs);
                    let rhs_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    self.switch_to(merge_bb);
                    let phi = self.func.new_value();
                    self.func.block_mut(merge_bb).phis.push(crate::mir::Phi {
                        dest: phi,
                        ty: Type::Bool,
                        incoming: vec![(cur_bb, false_val), (rhs_end, r)],
                    });
                    return Some(phi);
                }
                if *op == ast::BinOp::Or {
                    let l = self.lower_expr(lhs);
                    let true_val = self.emit(InstKind::BoolConst(true), Type::Bool, span);
                    let rhs_bb = self.new_block("or.rhs");
                    let merge_bb = self.new_block("or.merge");
                    let cur_bb = self.current_block;
                    self.set_terminator(Terminator::Branch(l, merge_bb, rhs_bb));
                    self.switch_to(rhs_bb);
                    let r = self.lower_expr(rhs);
                    let rhs_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    self.switch_to(merge_bb);
                    let phi = self.func.new_value();
                    self.func.block_mut(merge_bb).phis.push(crate::mir::Phi {
                        dest: phi,
                        ty: Type::Bool,
                        incoming: vec![(cur_bb, true_val), (rhs_end, r)],
                    });
                    return Some(phi);
                }
                let l = self.lower_expr(lhs);
                let r = self.lower_expr(rhs);
                let operand_ty = lhs.ty.clone();
                match op {
                    ast::BinOp::Eq => {
                        self.emit(InstKind::Cmp(CmpOp::Eq, l, r, operand_ty.clone()), ty, span)
                    }
                    ast::BinOp::Ne => {
                        self.emit(InstKind::Cmp(CmpOp::Ne, l, r, operand_ty.clone()), ty, span)
                    }
                    ast::BinOp::Lt => {
                        self.emit(InstKind::Cmp(CmpOp::Lt, l, r, operand_ty.clone()), ty, span)
                    }
                    ast::BinOp::Gt => {
                        self.emit(InstKind::Cmp(CmpOp::Gt, l, r, operand_ty.clone()), ty, span)
                    }
                    ast::BinOp::Le => {
                        self.emit(InstKind::Cmp(CmpOp::Le, l, r, operand_ty.clone()), ty, span)
                    }
                    ast::BinOp::Ge => {
                        self.emit(InstKind::Cmp(CmpOp::Ge, l, r, operand_ty), ty, span)
                    }
                    _ => {
                        let mir_op = super::lower_binop(op);
                        self.emit(InstKind::BinOp(mir_op, l, r), ty, span)
                    }
                }
            }

            ExprKind::UnaryOp(op, inner) => {
                let v = self.lower_expr(inner);
                let mir_op = super::lower_unaryop(op);
                self.emit(InstKind::UnaryOp(mir_op, v), ty, span)
            }

            ExprKind::Call(_, name, args) => {
                let arg_vals: Vec<ValueId> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call(name.clone(), arg_vals), ty, span)
            }

            ExprKind::IndirectCall(callee, args) => {
                let f = self.lower_expr(callee);
                let arg_vals: Vec<ValueId> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::IndirectCall(f, arg_vals), ty, span)
            }

            // Method(obj, mangled_name, plain_method_name, args)
            ExprKind::Method(obj, mangled_name, _method_name, args) => {
                let obj_val = self.lower_expr(obj);
                let arg_vals: Vec<ValueId> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(
                    InstKind::MethodCall(obj_val, mangled_name.clone(), arg_vals),
                    ty,
                    span,
                )
            }

            ExprKind::Field(obj, field, _idx) => {
                let obj_val = self.lower_expr(obj);
                self.emit(InstKind::FieldGet(obj_val, field.clone()), ty, span)
            }

            ExprKind::Index(arr, idx) => {
                let a = self.lower_expr(arr);
                let i = self.lower_expr(idx);
                self.emit(InstKind::Index(a, i), ty, span)
            }

            ExprKind::Struct(name, inits) => {
                let fields: Vec<(Symbol, ValueId)> = inits
                    .iter()
                    .map(|fi| {
                        let v = self.lower_expr(&fi.value);
                        (fi.name.unwrap_or(Symbol::intern("")), v)
                    })
                    .collect();
                self.emit(InstKind::StructInit(*name, fields), ty, span)
            }

            ExprKind::VariantCtor(enum_name, variant_name, tag, inits) => {
                let arg_vals: Vec<ValueId> =
                    inits.iter().map(|fi| self.lower_expr(&fi.value)).collect();
                self.emit(
                    InstKind::VariantInit(*enum_name, *variant_name, *tag, arg_vals),
                    ty,
                    span,
                )
            }

            ExprKind::IfExpr(if_expr) => {
                // Demote variables assigned in branches to memory
                // so the merge point reads current values via Load.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&if_expr.then, &mut assigned);
                for (_, elif_body) in &if_expr.elifs {
                    Self::collect_assigned_vars(elif_body, &mut assigned);
                }
                if let Some(els) = &if_expr.els {
                    Self::collect_assigned_vars(els, &mut assigned);
                }
                let pre_existing: HashSet<Symbol> = assigned
                    .iter()
                    .filter(|n| self.var_map.contains_key(*n))
                    .cloned()
                    .collect();
                self.demote_vars_to_memory(&pre_existing, span);

                // Variables first defined in BOTH then and else → promote to mem_vars
                if if_expr.els.is_some() || !if_expr.elifs.is_empty() {
                    let mut then_binds = HashSet::new();
                    Self::collect_new_binds(&if_expr.then, &mut then_binds);
                    let mut other_binds = HashSet::new();
                    for (_, elif_body) in &if_expr.elifs {
                        Self::collect_new_binds(elif_body, &mut other_binds);
                    }
                    if let Some(els) = &if_expr.els {
                        Self::collect_new_binds(els, &mut other_binds);
                    }
                    for name in then_binds.intersection(&other_binds) {
                        if !self.var_map.contains_key(name) && !self.mem_vars.contains(name) {
                            self.mem_vars.insert(name.clone());
                        }
                    }
                }

                let cond_val = self.lower_expr(&if_expr.cond);
                let then_bb = self.new_block("if.then");
                let merge_bb = self.new_block("if.merge");

                // Determine the false-branch target:
                // elif chain first, then else, then merge.
                let first_elif_bb = if !if_expr.elifs.is_empty() {
                    Some(self.new_block("elif.test"))
                } else {
                    None
                };
                let else_bb = if if_expr.els.is_some() && first_elif_bb.is_none() {
                    self.new_block("if.else")
                } else {
                    first_elif_bb.unwrap_or(merge_bb)
                };

                self.set_terminator(Terminator::Branch(cond_val, then_bb, else_bb));

                // Then branch
                self.switch_to(then_bb);
                let then_val = self.lower_block_expr(&if_expr.then);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                // Lower elif chains
                let mut elif_vals: Vec<(BlockId, ValueId)> = Vec::new();
                let mut prev_false_bb = first_elif_bb;
                for (i, (elif_cond, elif_body)) in if_expr.elifs.iter().enumerate() {
                    let elif_test = prev_false_bb.unwrap();
                    let elif_body_bb = self.new_block("elif.body");

                    let is_last_elif = i + 1 == if_expr.elifs.len();
                    let elif_false_bb = if is_last_elif {
                        if if_expr.els.is_some() {
                            Some(self.new_block("if.else"))
                        } else {
                            None
                        }
                    } else {
                        Some(self.new_block("elif.test"))
                    };

                    self.switch_to(elif_test);
                    let c = self.lower_expr(elif_cond);
                    self.set_terminator(Terminator::Branch(
                        c,
                        elif_body_bb,
                        elif_false_bb.unwrap_or(merge_bb),
                    ));

                    self.switch_to(elif_body_bb);
                    let elif_val = self.lower_block_expr(elif_body);
                    let elif_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    elif_vals.push((elif_end, elif_val));

                    prev_false_bb = elif_false_bb;
                }

                // Else branch
                let else_val_info = if let Some(els) = &if_expr.els {
                    let else_target = prev_false_bb.unwrap_or(else_bb);
                    self.switch_to(else_target);
                    let else_val = self.lower_block_expr(els);
                    let else_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    Some((else_end, else_val))
                } else {
                    None
                };

                // Merge
                self.switch_to(merge_bb);
                if !matches!(ty, Type::Void) && (else_val_info.is_some() || !elif_vals.is_empty()) {
                    let mut incoming = vec![(then_end, then_val)];
                    for &(bb, v) in &elif_vals {
                        incoming.push((bb, v));
                    }
                    if let Some((eb, ev)) = else_val_info {
                        incoming.push((eb, ev));
                    }
                    // If no else branch, add a void from the last false branch
                    if else_val_info.is_none() && elif_vals.is_empty() {
                        // No phi needed — only then branch produces a value
                    }
                    let result = self.new_value();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest: result,
                        ty: ty.clone(),
                        incoming,
                    });
                    result
                } else if !matches!(ty, Type::Void) {
                    // No elif/else — no phi, just pass through then value
                    // via a phi from then or a void from merge
                    let void_val = self.emit(InstKind::Void, Type::Void, span);
                    let result = self.new_value();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest: result,
                        ty: ty.clone(),
                        incoming: vec![(then_end, then_val), (self.current_block, void_val)],
                    });
                    result
                } else {
                    self.emit(InstKind::Void, Type::Void, span)
                }
            }

            ExprKind::Ternary(cond, then_expr, else_expr) => {
                let cond_val = self.lower_expr(cond);
                let then_bb = self.new_block("ternary.then");
                let else_bb = self.new_block("ternary.else");
                let merge_bb = self.new_block("ternary.merge");

                self.set_terminator(Terminator::Branch(cond_val, then_bb, else_bb));

                self.switch_to(then_bb);
                let then_val = self.lower_expr(then_expr);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(else_bb);
                let else_val = self.lower_expr(else_expr);
                let else_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(merge_bb);
                let result = self.new_value();
                self.func.block_mut(merge_bb).phis.push(Phi {
                    dest: result,
                    ty: ty.clone(),
                    incoming: vec![(then_end, then_val), (else_end, else_val)],
                });
                result
            }

            ExprKind::Cast(inner, target_ty) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Cast(v, target_ty.clone()), ty, span)
            }
            ExprKind::StrictCast(inner, target_ty) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::StrictCast(v, target_ty.clone()), ty, span)
            }

            ExprKind::Ref(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Ref(v), ty, span)
            }

            ExprKind::Deref(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Deref(v), ty, span)
            }

            ExprKind::Array(elems) | ExprKind::Tuple(elems) => {
                let vals: Vec<ValueId> = elems.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(InstKind::ArrayInit(vals), ty, span)
            }

            ExprKind::Slice(arr, start, end) => {
                let a = self.lower_expr(arr);
                let s = self.lower_expr(start);
                let e = self.lower_expr(end);
                self.emit(InstKind::Slice(a, s, e), ty, span)
            }

            ExprKind::FnRef(_, name) => self.emit(InstKind::FnRef(*name), ty, span),

            _ => return None,
        })
    }
}

impl Lowerer {
    pub(super) fn lower_block_expr(&mut self, stmts: &[hir::Stmt]) -> ValueId {
        let mut last = self.emit(InstKind::Void, Type::Void, Span::dummy());
        for stmt in stmts {
            last = self.lower_stmt(stmt);
        }
        last
    }
}
