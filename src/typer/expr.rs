use std::path::PathBuf;

use crate::ast::{self, BinOp, Span, UnaryOp};
use crate::hir::{self, CoercionKind, DefId, Ownership};
use crate::types::Type;

use super::{Typer, VarInfo};

impl Typer {
    pub(crate) fn lower_expr(&mut self, expr: &ast::Expr) -> Result<hir::Expr, String> {
        self.lower_expr_expected(expr, None)
    }

    pub(crate) fn lower_expr_expected(
        &mut self,
        expr: &ast::Expr,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        if let ast::Expr::Lambda(params, ret, body, span) = expr {
            return self.lower_lambda_with_expected(params, ret, body, *span, expected);
        }

        if let ast::Expr::Array(elems, span) = expr {
            let expected_elem = match expected {
                Some(Type::Array(et, _)) => Some(et.as_ref()),
                _ => None,
            };
            let helems: Vec<hir::Expr> = elems
                .iter()
                .map(|e| self.lower_expr_expected(e, expected_elem))
                .collect::<Result<_, _>>()?;
            let et = helems
                .first()
                .map(|e| e.ty.clone())
                .or_else(|| expected_elem.cloned())
                .unwrap_or_else(|| self.infer_ctx.fresh_var());
            for elem in helems.iter().skip(1) {
                let _ = self
                    .infer_ctx
                    .unify_at(&et, &elem.ty, *span, "array element");
            }
            let len = helems.len();
            return Ok(hir::Expr {
                kind: hir::ExprKind::Array(helems),
                ty: Type::Array(Box::new(et), len),
                span: *span,
            });
        }

        match expr {
            ast::Expr::Int(n, span) => {
                let ty = match expected {
                    Some(t) if t.is_int() => t.clone(),
                    Some(t) => {
                        // Expected type is not a concrete int — create a fresh integer
                        // TypeVar and unify with expected to propagate constraints
                        let fresh = self.infer_ctx.fresh_integer_var();
                        let _ = self.infer_ctx.unify(&fresh, t);
                        fresh
                    }
                    None => self.infer_ctx.fresh_integer_var(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Int(*n),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Float(n, span) => {
                let ty = match expected {
                    Some(t) if t.is_float() => t.clone(),
                    Some(t) => {
                        let fresh = self.infer_ctx.fresh_float_var();
                        let _ = self.infer_ctx.unify(&fresh, t);
                        fresh
                    }
                    None => self.infer_ctx.fresh_float_var(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Float(*n),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Str(s, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Str(s.clone()),
                ty: Type::String,
                span: *span,
            }),

            ast::Expr::Bool(v, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Bool(*v),
                ty: Type::Bool,
                span: *span,
            }),

            ast::Expr::None(span) => {
                let ty = expected
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var());
                Ok(hir::Expr {
                    kind: hir::ExprKind::None,
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Void(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: Type::Void,
                span: *span,
            }),

            ast::Expr::Ident(name, span) => {
                if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
                    let is_unit = self
                        .enums
                        .get(&enum_name)
                        .and_then(|vs| vs.iter().find(|(vn, _)| vn == name))
                        .map(|(_, fs)| fs.is_empty())
                        .unwrap_or(false);
                    if is_unit {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(enum_name.clone(), name.clone(), tag),
                            ty: Type::Enum(enum_name),
                            span: *span,
                        });
                    }
                    if let Ok(Some(_mangled)) = self.try_monomorphize_generic_variant_bare(name) {
                        let (en2, tag2) = self
                            .variant_tags
                            .get(name)
                            .cloned()
                            .unwrap_or((enum_name.clone(), tag));
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(en2.clone(), name.clone(), tag2),
                            ty: Type::Enum(en2),
                            span: *span,
                        });
                    }
                } else if let Ok(Some(mangled)) = self.try_monomorphize_generic_variant_bare(name) {
                    if let Some((_, tag)) = self.variant_tags.get(name).cloned() {
                        return Ok(hir::Expr {
                            kind: hir::ExprKind::VariantRef(mangled.clone(), name.clone(), tag),
                            ty: Type::Enum(mangled),
                            span: *span,
                        });
                    }
                }
                if let Some(v) = self.find_var(name) {
                    let def_id = v.def_id;
                    let mono_ty = v.ty.clone();
                    let scheme_clone = v.scheme.clone();
                    // Drop v borrow before calling instantiate
                    let ty = match (&scheme_clone, expected) {
                        // R1.1 FIX: Always instantiate poly schemes — never unify
                        // with the original mono type. The original body's TypeVars
                        // were defaulted by default_quantified_vars() after
                        // generalization. Each use site gets fresh TypeVars via
                        // instantiation, which are then unified with the expected
                        // type. This fixes the "first-call-wins" bug where the
                        // first call site would permanently commit the original
                        // TypeVars, preventing polymorphic multi-use.
                        (Some(scheme), Some(exp)) if scheme.is_poly() => {
                            let inst = self.infer_ctx.instantiate(scheme);
                            let _ = self.infer_ctx.unify(&inst, exp);
                            inst
                        }
                        (Some(scheme), _) if scheme.is_poly() => self.infer_ctx.instantiate(scheme),
                        _ => mono_ty,
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Var(def_id, name.clone()),
                        ty,
                        span: *span,
                    });
                }
                if let Some(const_expr) = self.consts.get(name).cloned() {
                    return self.lower_expr(&const_expr);
                }
                if let Some((id, ptys, ret)) = self.fns.get(name).cloned() {
                    // If the function has a poly scheme, instantiate it when used
                    // as a value so that the TypeVars are fresh for this usage site
                    let fn_ty =
                        if let Some((ref q, ref sp, ref sr)) = self.fn_schemes.get(name).cloned() {
                            if !q.is_empty() {
                                let scheme = crate::types::Scheme {
                                    quantified: q.clone(),
                                    ty: Type::Fn(sp.clone(), Box::new(sr.clone())),
                                };
                                self.infer_ctx.instantiate(&scheme)
                            } else {
                                Type::Fn(ptys, Box::new(ret))
                            }
                        } else {
                            Type::Fn(ptys, Box::new(ret))
                        };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::FnRef(id, name.clone()),
                        ty: fn_ty,
                        span: *span,
                    });
                }
                // Check generic fns so ident references to them don't error
                if self.generic_fns.contains_key(name) {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Var(DefId::BUILTIN, name.clone()),
                        ty: self.infer_ctx.fresh_var(),
                        span: *span,
                    });
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::Var(DefId::BUILTIN, name.clone()),
                    ty: self.infer_ctx.fresh_var(),
                    span: *span,
                })
            }

            ast::Expr::BinOp(lhs, op, rhs, span) => {
                let hl = self.lower_expr(lhs)?;
                let hr = self.lower_expr_expected(rhs, Some(&hl.ty))?;
                // Unify operand types for numeric consistency
                let r = self
                    .infer_ctx
                    .unify_at(&hl.ty, &hr.ty, *span, "binary operands");
                self.collect_unify_error(r);

                // Phase 4 (P2): Add type constraints based on operator kind.
                // Arithmetic ops require numeric types, bitwise ops require integer types.
                // Skip constraint if operand is already resolved to String (concat) or
                // Struct (operator overloading) — only constrain unsolved TypeVars.
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Exp => {
                        let resolved_l = self.infer_ctx.shallow_resolve(&hl.ty);
                        // Only add Numeric constraint if the operand type is still a TypeVar
                        // (unresolved). String+ is concat, Struct+ is overloaded — don't constrain those.
                        if matches!(resolved_l, Type::TypeVar(_)) {
                            let _ = self.infer_ctx.constrain(
                                &hl.ty,
                                super::unify::TypeConstraint::Numeric,
                                *span,
                                "arithmetic operator requires numeric type",
                            );
                        }
                        let resolved_r = self.infer_ctx.shallow_resolve(&hr.ty);
                        if matches!(resolved_r, Type::TypeVar(_)) {
                            let _ = self.infer_ctx.constrain(
                                &hr.ty,
                                super::unify::TypeConstraint::Numeric,
                                *span,
                                "arithmetic operator requires numeric type",
                            );
                        }
                    }
                    BinOp::Shl | BinOp::Shr | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => {
                        let _ = self.infer_ctx.constrain(
                            &hl.ty,
                            super::unify::TypeConstraint::Integer,
                            *span,
                            "bitwise operator requires integer type",
                        );
                        let _ = self.infer_ctx.constrain(
                            &hr.ty,
                            super::unify::TypeConstraint::Integer,
                            *span,
                            "bitwise operator requires integer type",
                        );
                    }
                    BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                        // Comparison requires numeric operands (Jade has no non-numeric Ord)
                        let resolved_l = self.infer_ctx.shallow_resolve(&hl.ty);
                        if matches!(resolved_l, Type::TypeVar(_)) {
                            let _ = self.infer_ctx.constrain(
                                &hl.ty,
                                super::unify::TypeConstraint::Numeric,
                                *span,
                                "comparison operator requires numeric type",
                            );
                        }
                    }
                    // Eq/Ne work on all types, And/Or work on bools — no constraint needed
                    _ => {}
                }

                let (hl, hr) = self.coerce_binop_operands(hl, hr);
                let result_ty = match op {
                    BinOp::Eq
                    | BinOp::Ne
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::Le
                    | BinOp::Ge
                    | BinOp::And
                    | BinOp::Or => Type::Bool,
                    _ => hl.ty.clone(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::BinOp(Box::new(hl), *op, Box::new(hr)),
                    ty: result_ty,
                    span: *span,
                })
            }

            ast::Expr::UnaryOp(op, inner, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = match op {
                    UnaryOp::Not => {
                        // Logical not for Bool, bitwise not for integers
                        let resolved = self.infer_ctx.resolve(&hi.ty);
                        if resolved.is_int() {
                            hi.ty.clone()
                        } else {
                            Type::Bool
                        }
                    }
                    UnaryOp::Neg => {
                        // Negation requires numeric type
                        let _ = self.infer_ctx.constrain(
                            &hi.ty,
                            super::unify::TypeConstraint::Numeric,
                            *span,
                            "negation requires numeric type",
                        );
                        hi.ty.clone()
                    }
                    UnaryOp::BitNot => {
                        // Bitwise not requires integer type
                        let _ = self.infer_ctx.constrain(
                            &hi.ty,
                            super::unify::TypeConstraint::Integer,
                            *span,
                            "bitwise not requires integer type",
                        );
                        hi.ty.clone()
                    }
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::UnaryOp(*op, Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Call(callee, args, span) => {
                let result = self.lower_call(callee, args, *span)?;
                if let Some(exp) = expected {
                    let _ = self
                        .infer_ctx
                        .unify_at(exp, &result.ty, *span, "call result");
                }
                Ok(result)
            }

            ast::Expr::Method(obj, method, args, span) => {
                let result = self.lower_method_call(obj, method, args, *span)?;
                if let Some(exp) = expected {
                    let _ = self
                        .infer_ctx
                        .unify_at(exp, &result.ty, *span, "method call result");
                }
                Ok(result)
            }

            ast::Expr::Field(obj, field, span) => {
                let hobj = self.lower_expr(obj)?;
                let resolved_ty = self.infer_ctx.shallow_resolve(&hobj.ty);
                let struct_name = match &resolved_ty {
                    Type::Struct(name) => Some(name.clone()),
                    Type::Ptr(inner) => match inner.as_ref() {
                        Type::Struct(name) => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                let (ty, idx) = if let Some(ref name) = struct_name {
                    if let Some(fields) = self.structs.get(name) {
                        if let Some((i, (_, fty))) =
                            fields.iter().enumerate().find(|(_, (n, _))| n == field)
                        {
                            (self.infer_ctx.shallow_resolve(fty), i)
                        } else {
                            return Err(format!(
                                "line {}:{}: type '{}' has no field '{}'",
                                span.line, span.col, name, field
                            ));
                        }
                    } else {
                        (Type::I64, 0)
                    }
                } else if matches!(resolved_ty, Type::String) && field == "length" {
                    (Type::I64, 0)
                } else if matches!(&resolved_ty, Type::Vec(_))
                    && (field == "length" || field == "len")
                {
                    (Type::I64, 0)
                } else if matches!(&resolved_ty, Type::Map(_, _))
                    && (field == "length" || field == "len")
                {
                    (Type::I64, 0)
                } else if let Type::Tuple(ref tys) = resolved_ty {
                    // Tuple field access: .0, .1, etc.
                    if let Ok(idx) = field.parse::<usize>() {
                        if idx < tys.len() {
                            (tys[idx].clone(), idx)
                        } else {
                            return Err(format!(
                                "line {}:{}: tuple index {} out of range (tuple has {} elements)",
                                span.line,
                                span.col,
                                idx,
                                tys.len()
                            ));
                        }
                    } else {
                        (self.infer_ctx.fresh_var(), 0)
                    }
                } else if matches!(resolved_ty, Type::TypeVar(_)) {
                    // Structural field constraint: record field access on this TypeVar.
                    // Search all known structs that have this field, and filter by
                    // any previously-recorded field constraints on the same TypeVar.
                    let var_id = if let Type::TypeVar(v) = resolved_ty {
                        self.infer_ctx.find(v)
                    } else {
                        0
                    };

                    // Record this field constraint
                    let fty_placeholder = self.infer_ctx.fresh_var();
                    self.field_constraints
                        .entry(var_id)
                        .or_default()
                        .push((field.clone(), fty_placeholder.clone()));

                    // Collect ALL field constraints on this TypeVar
                    let all_required_fields: Vec<(String, Type)> = self
                        .field_constraints
                        .get(&var_id)
                        .cloned()
                        .unwrap_or_default();

                    // Find structs satisfying ALL field constraints
                    let candidates: Vec<(String, Vec<(String, Type, usize)>)> = self
                        .structs
                        .iter()
                        .filter_map(|(sname, fields)| {
                            let mut matched = Vec::new();
                            for (req_name, _) in &all_required_fields {
                                if let Some((idx, (_, fty))) = fields
                                    .iter()
                                    .enumerate()
                                    .find(|(_, (fname, _))| fname == req_name)
                                {
                                    matched.push((req_name.clone(), fty.clone(), idx));
                                } else {
                                    return None; // struct missing a required field
                                }
                            }
                            Some((sname.clone(), matched))
                        })
                        .collect();

                    if candidates.len() == 1 {
                        // Unique match — constrain the TypeVar to this struct
                        let (sname, matched_fields) = &candidates[0];
                        let struct_ty = Type::Struct(sname.clone());
                        let _ = self.infer_ctx.unify_at(
                            &resolved_ty,
                            &struct_ty,
                            *span,
                            "field access implies struct type",
                        );
                        // Unify all recorded field TypeVars with actual field types
                        for (req_name, req_ty) in &all_required_fields {
                            if let Some((_, actual_ty, _)) =
                                matched_fields.iter().find(|(n, _, _)| n == req_name)
                            {
                                let actual_resolved = self.infer_ctx.shallow_resolve(actual_ty);
                                let _ = self.infer_ctx.unify_at(
                                    req_ty,
                                    &actual_resolved,
                                    *span,
                                    "struct field type",
                                );
                            }
                        }
                        // Find the index for the current field
                        let (fty, idx) = matched_fields
                            .iter()
                            .find(|(n, _, _)| n == field)
                            .map(|(_, t, i)| (self.infer_ctx.shallow_resolve(t), *i))
                            .unwrap_or_else(|| (fty_placeholder, 0));
                        (fty, idx)
                    } else {
                        // Ambiguous or no match — defer
                        self.deferred_fields.push(super::DeferredField {
                            receiver_ty: resolved_ty.clone(),
                            field_name: field.clone(),
                            field_ty: fty_placeholder.clone(),
                            span: *span,
                        });
                        (fty_placeholder, 0)
                    }
                } else {
                    (self.infer_ctx.fresh_var(), 0)
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Field(Box::new(hobj), field.clone(), idx),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Index(arr, idx, span) => {
                let harr = self.lower_expr(arr)?;
                let hidx = self.lower_expr(idx)?;
                let elem_ty = match &harr.ty {
                    Type::Array(et, _) => *et.clone(),
                    Type::Vec(et) => *et.clone(),
                    Type::Ptr(et) => *et.clone(),
                    Type::Map(_, vt) => *vt.clone(),
                    Type::Tuple(tys) => tys
                        .first()
                        .cloned()
                        .unwrap_or_else(|| self.infer_ctx.fresh_var()),
                    _ => self.infer_ctx.fresh_var(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Index(Box::new(harr), Box::new(hidx)),
                    ty: elem_ty,
                    span: *span,
                })
            }

            ast::Expr::Ternary(cond, then, els, span) => {
                let hc = self.lower_expr(cond)?;
                let ht = self.lower_expr_expected(then, expected)?;
                let he = self.lower_expr_expected(els, expected)?;
                let _ = self
                    .infer_ctx
                    .unify_at(&ht.ty, &he.ty, *span, "ternary branches");
                let ty = ht.ty.clone();
                let he = self.maybe_coerce_to(he, &ty);
                Ok(hir::Expr {
                    kind: hir::ExprKind::Ternary(Box::new(hc), Box::new(ht), Box::new(he)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::As(inner, target_ty, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = target_ty.clone();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Cast(Box::new(hi), ty.clone()),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Array(elems, span) => {
                let helems: Vec<hir::Expr> = elems
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                let et = helems
                    .first()
                    .map(|e| e.ty.clone())
                    .unwrap_or_else(|| self.infer_ctx.fresh_var());
                for elem in helems.iter().skip(1) {
                    let _ = self
                        .infer_ctx
                        .unify_at(&et, &elem.ty, *span, "array element");
                }
                let len = helems.len();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Array(helems),
                    ty: Type::Array(Box::new(et), len),
                    span: *span,
                })
            }

            ast::Expr::Tuple(elems, span) => {
                let helems: Vec<hir::Expr> = elems
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                let tys: Vec<Type> = helems.iter().map(|e| e.ty.clone()).collect();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Tuple(helems),
                    ty: Type::Tuple(tys),
                    span: *span,
                })
            }

            ast::Expr::Struct(name, inits, span) => {
                self.lower_struct_or_variant(name, inits, *span)
            }

            ast::Expr::IfExpr(i) => {
                let result_ty = expected
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var());
                let hi = self.lower_if(i, &result_ty)?;
                let ty = match hi.then.last() {
                    Some(hir::Stmt::Expr(e)) => e.ty.clone(),
                    _ => Type::Void,
                };
                if let Some(ref els) = hi.els {
                    if let Some(hir::Stmt::Expr(e)) = els.last() {
                        let r =
                            self.infer_ctx
                                .unify_at(&ty, &e.ty, i.span, "if-expression branches");
                        self.collect_unify_error(r);
                    }
                }
                for (_, branch) in &hi.elifs {
                    if let Some(hir::Stmt::Expr(e)) = branch.last() {
                        let r = self.infer_ctx.unify_at(&ty, &e.ty, i.span, "elif branch");
                        self.collect_unify_error(r);
                    }
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::IfExpr(Box::new(hi)),
                    ty,
                    span: i.span,
                })
            }

            ast::Expr::Pipe(left, right, extra_args, span) => {
                self.lower_pipe(left, right, extra_args, *span)
            }

            ast::Expr::Block(stmts, span) => {
                // Phase 4: Lower all statements, but pass expected into the tail expression
                self.push_scope();
                let mut hstmts = Vec::new();
                let len = stmts.len();
                for (i, s) in stmts.iter().enumerate() {
                    if i == len - 1 {
                        // Tail statement: if it's an expression, use expected type
                        if let ast::Stmt::Expr(e) = s {
                            let he = self.lower_expr_expected(e, expected)?;
                            hstmts.push(hir::Stmt::Expr(he));
                        } else {
                            hstmts.push(self.lower_stmt(s, &Type::Void)?);
                        }
                    } else {
                        hstmts.push(self.lower_stmt(s, &Type::Void)?);
                    }
                }
                self.pop_scope();
                let ty = match hstmts.last() {
                    Some(hir::Stmt::Expr(e)) => e.ty.clone(),
                    _ => Type::Void,
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Block(hstmts),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Lambda(params, ret, body, span) => {
                self.lower_lambda_with_expected(params, ret, body, *span, expected)
            }

            ast::Expr::Placeholder(span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: expected
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var()),
                span: *span,
            }),

            ast::Expr::Ref(inner, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = Type::Ptr(Box::new(hi.ty.clone()));
                Ok(hir::Expr {
                    kind: hir::ExprKind::Ref(Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Deref(inner, span) => {
                let hi = self.lower_expr(inner)?;
                let resolved = self.infer_ctx.shallow_resolve(&hi.ty);
                let ty = match &resolved {
                    Type::Ptr(inner_ty) | Type::Rc(inner_ty) => *inner_ty.clone(),
                    Type::TypeVar(_) => {
                        // Deref on unsolved type: create inner TypeVar, constrain outer as Ptr
                        let inner_var = self.infer_ctx.fresh_var();
                        let ptr_ty = Type::Ptr(Box::new(inner_var.clone()));
                        let _ = self
                            .infer_ctx
                            .unify_at(&resolved, &ptr_ty, *span, "dereference");
                        inner_var
                    }
                    other => {
                        return Err(format!(
                            "line {}:{}: cannot dereference type `{}`",
                            span.line, span.col, other
                        ));
                    }
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Deref(Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::ListComp(body_expr, var, iter_expr, iter_end, cond, span) => {
                let hiter = self.lower_expr(iter_expr)?;

                // When iter_end is Some, this is a range comprehension (e.g., 0 to 5)
                // and the bind type is I64, same as for-loops.
                let is_range = iter_end.is_some();
                let bind_ty = if is_range {
                    Type::I64
                } else {
                    match &hiter.ty {
                        Type::Array(et, _) | Type::Ptr(et) => *et.clone(),
                        Type::Vec(et) => *et.clone(),
                        _ => self.infer_ctx.fresh_var(),
                    }
                };
                let bind_id = self.fresh_id();
                self.push_scope();
                self.define_var(
                    var,
                    VarInfo {
                        def_id: bind_id,
                        ty: bind_ty,
                        ownership: Ownership::Owned,
                        scheme: None,
                    },
                );
                let hbody = self.lower_expr(body_expr)?;
                // iter_end is the range end (e.g., `5` in `0 to 5`)
                let hend = iter_end.as_ref().map(|c| self.lower_expr(c)).transpose()?;
                let hcond = cond
                    .as_ref()
                    .map(|m| self.lower_expr_expected(m, Some(&Type::Bool)))
                    .transpose()?;
                self.pop_scope();

                let ty = Type::Ptr(Box::new(hbody.ty.clone()));
                Ok(hir::Expr {
                    kind: hir::ExprKind::ListComp(
                        Box::new(hbody),
                        bind_id,
                        var.clone(),
                        Box::new(hiter),
                        hend.map(Box::new),
                        hcond.map(Box::new),
                    ),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::Syscall(args, span) => {
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Syscall(hargs),
                    ty: Type::I64,
                    span: *span,
                })
            }

            ast::Expr::Embed(path, span) => {
                let base = self
                    .source_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("."));
                let file_path = base.join(path);
                let contents = std::fs::read_to_string(&file_path)
                    .map_err(|e| format!("embed '{}': {}", file_path.display(), e))?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Str(contents),
                    ty: Type::String,
                    span: *span,
                })
            }
            ast::Expr::Query(_, _, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: Type::Void,
                span: *span,
            }),

            ast::Expr::StoreQuery(store, filter, span) => {
                let schema = self
                    .store_schemas
                    .get(store)
                    .ok_or_else(|| format!("unknown store '{store}'"))?
                    .clone();
                let hfilter = self.lower_store_filter(filter, &schema, store)?;
                let struct_name = format!("__store_{store}");
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreQuery(store.clone(), Box::new(hfilter)),
                    ty: Type::Struct(struct_name),
                    span: *span,
                })
            }

            ast::Expr::StoreCount(store, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreCount(store.clone()),
                    ty: Type::I64,
                    span: *span,
                })
            }

            ast::Expr::StoreAll(store, span) => {
                if !self.store_schemas.contains_key(store) {
                    return Err(format!("unknown store '{store}'"));
                }
                let struct_name = format!("__store_{store}");
                Ok(hir::Expr {
                    kind: hir::ExprKind::StoreAll(store.clone()),
                    ty: Type::Ptr(Box::new(Type::Struct(struct_name))),
                    span: *span,
                })
            }

            ast::Expr::Spawn(name, span) => {
                if !self.actors.contains_key(name) {
                    return Err(format!("spawn: unknown actor '{name}'"));
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::Spawn(name.clone()),
                    ty: Type::ActorRef(name.clone()),
                    span: *span,
                })
            }

            ast::Expr::Send(target, handler, args, span) => {
                let htarget = self.lower_expr(target)?;
                let actor_name = match &htarget.ty {
                    Type::ActorRef(name) => name.clone(),
                    _ => {
                        return Err(format!(
                            "send: target must be an ActorRef, got {}",
                            htarget.ty
                        ));
                    }
                };
                let (_, _, ref handlers) = self
                    .actors
                    .get(&actor_name)
                    .ok_or_else(|| format!("send: unknown actor '{actor_name}'"))?
                    .clone();
                let (_, _, tag) =
                    handlers
                        .iter()
                        .find(|(n, _, _)| n == handler)
                        .ok_or_else(|| {
                            format!("send: actor '{actor_name}' has no handler '@{handler}'")
                        })?;
                let tag = *tag;
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .map(|e| self.lower_expr(e))
                    .collect::<Result<_, _>>()?;
                Ok(hir::Expr {
                    kind: hir::ExprKind::Send(
                        Box::new(htarget),
                        actor_name,
                        handler.clone(),
                        tag,
                        hargs,
                    ),
                    ty: Type::Void,
                    span: *span,
                })
            }

            ast::Expr::Receive(_, span) => Ok(hir::Expr {
                kind: hir::ExprKind::Void,
                ty: Type::Void,
                span: *span,
            }),

            ast::Expr::Yield(inner, span) => {
                let hi = self.lower_expr(inner)?;
                let ty = hi.ty.clone();
                Ok(hir::Expr {
                    kind: hir::ExprKind::Yield(Box::new(hi)),
                    ty,
                    span: *span,
                })
            }

            ast::Expr::DispatchBlock(name, body, span) => {
                let hbody = self.lower_block_no_scope(body, &Type::Void)?;
                let yield_ty = self.infer_coroutine_yield_type(&hbody);
                let coro_ty = Type::Coroutine(Box::new(yield_ty));
                if name != "__anon" {
                    let id = self.fresh_id();
                    self.define_var(
                        name,
                        VarInfo {
                            def_id: id,
                            ty: coro_ty.clone(),
                            ownership: crate::hir::Ownership::Owned,
                            scheme: None,
                        },
                    );
                }
                Ok(hir::Expr {
                    kind: hir::ExprKind::CoroutineCreate(name.clone(), hbody),
                    ty: coro_ty,
                    span: *span,
                })
            }

            ast::Expr::ChannelCreate(elem_ty, cap, span) => {
                let hcap = self.lower_expr(cap)?;
                let resolved_elem_ty = match elem_ty {
                    Some(ty) => ty.clone(),
                    None => self.infer_ctx.fresh_var(),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::ChannelCreate(resolved_elem_ty.clone(), Box::new(hcap)),
                    ty: Type::Channel(Box::new(resolved_elem_ty)),
                    span: *span,
                })
            }

            ast::Expr::ChannelSend(ch, val, span) => {
                let hch = self.lower_expr(ch)?;
                let resolved_ch_ty = self.infer_ctx.shallow_resolve(&hch.ty);
                let elem_ty = match &resolved_ch_ty {
                    Type::Channel(t) => (**t).clone(),
                    Type::TypeVar(_) => {
                        // R2.2: Channel type not yet known — create a fresh elem
                        // TypeVar and constrain the channel to Channel<elem_ty>
                        let elem_var = self.infer_ctx.fresh_var();
                        let chan_ty = Type::Channel(Box::new(elem_var.clone()));
                        let _ = self.infer_ctx.unify_at(
                            &resolved_ch_ty,
                            &chan_ty,
                            *span,
                            "channel send infers channel type",
                        );
                        elem_var
                    }
                    _ => return Err(format!("send: target must be a Channel, got {}", hch.ty)),
                };
                let hval = self.lower_expr(val)?;
                let _ = self
                    .infer_ctx
                    .unify_at(&elem_ty, &hval.ty, *span, "channel send");
                let hval = self.maybe_coerce_to(hval, &elem_ty);
                Ok(hir::Expr {
                    kind: hir::ExprKind::ChannelSend(Box::new(hch), Box::new(hval)),
                    ty: Type::Void,
                    span: *span,
                })
            }

            ast::Expr::ChannelRecv(ch, span) => {
                let hch = self.lower_expr(ch)?;
                let resolved_ch_ty = self.infer_ctx.shallow_resolve(&hch.ty);
                let elem_ty = match &resolved_ch_ty {
                    Type::Channel(t) => (**t).clone(),
                    Type::TypeVar(_) => {
                        // R2.2: Channel type not yet known — create a fresh elem
                        // TypeVar and constrain the channel to Channel<elem_ty>
                        let elem_var = self.infer_ctx.fresh_var();
                        let chan_ty = Type::Channel(Box::new(elem_var.clone()));
                        let _ = self.infer_ctx.unify_at(
                            &resolved_ch_ty,
                            &chan_ty,
                            *span,
                            "channel recv infers channel type",
                        );
                        elem_var
                    }
                    _ => return Err(format!("receive: target must be a Channel, got {}", hch.ty)),
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::ChannelRecv(Box::new(hch)),
                    ty: elem_ty,
                    span: *span,
                })
            }

            ast::Expr::Select(arms, default_body, span) => {
                let mut harms = Vec::new();
                for arm in arms {
                    let hch = self.lower_expr(&arm.chan)?;
                    let resolved_sel_ch = self.infer_ctx.shallow_resolve(&hch.ty);
                    let elem_ty = match &resolved_sel_ch {
                        Type::Channel(t) => (**t).clone(),
                        Type::TypeVar(_) => {
                            let elem_var = self.infer_ctx.fresh_var();
                            let chan_ty = Type::Channel(Box::new(elem_var.clone()));
                            let _ = self.infer_ctx.unify_at(
                                &resolved_sel_ch,
                                &chan_ty,
                                arm.span,
                                "select infers channel type",
                            );
                            elem_var
                        }
                        _ => {
                            return Err(format!(
                                "select: channel must be a Channel type, got {}",
                                hch.ty
                            ));
                        }
                    };
                    let hval = if let Some(ref v) = arm.value {
                        let hv = self.lower_expr(v)?;
                        if arm.is_send {
                            let _ =
                                self.infer_ctx
                                    .unify_at(&elem_ty, &hv.ty, arm.span, "select send");
                        }
                        Some(hv)
                    } else {
                        None
                    };
                    let bind_id = arm.binding.as_ref().map(|_| self.fresh_id());
                    if let (Some(name), Some(id)) = (&arm.binding, bind_id) {
                        self.define_var(
                            name,
                            VarInfo {
                                def_id: id,
                                ty: elem_ty.clone(),
                                ownership: hir::Ownership::Owned,
                                scheme: None,
                            },
                        );
                    }
                    let hbody = self.lower_block_no_scope(&arm.body, &Type::Void)?;
                    harms.push(hir::SelectArm {
                        is_send: arm.is_send,
                        chan: hch,
                        value: hval,
                        binding: arm.binding.clone(),
                        bind_id,
                        elem_ty,
                        body: hbody,
                        span: arm.span,
                    });
                }
                let hdefault = if let Some(body) = default_body {
                    Some(self.lower_block_no_scope(body, &Type::Void)?)
                } else {
                    None
                };
                Ok(hir::Expr {
                    kind: hir::ExprKind::Select(harms, hdefault),
                    ty: Type::Void,
                    span: *span,
                })
            }
        }
    }

    fn lower_struct_or_variant(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
    ) -> Result<hir::Expr, String> {
        if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
            // Look up variant field types from enum definition
            let variant_fields: Vec<Type> = self
                .enums
                .get(&enum_name)
                .and_then(|vs| vs.iter().find(|(vn, _)| vn == name))
                .map(|(_, ftys)| ftys.clone())
                .unwrap_or_default();
            let hinits: Vec<hir::FieldInit> = inits
                .iter()
                .enumerate()
                .map(|(i, fi)| {
                    let expected = variant_fields.get(i);
                    Ok(hir::FieldInit {
                        name: fi.name.clone(),
                        value: self.lower_expr_expected(&fi.value, expected)?,
                    })
                })
                .collect::<Result<_, String>>()?;
            return Ok(hir::Expr {
                kind: hir::ExprKind::VariantCtor(enum_name.clone(), name.to_string(), tag, hinits),
                ty: Type::Enum(enum_name),
                span,
            });
        }

        // Lower inits with expected types from struct definition when available
        let struct_fields = self.structs.get(name).cloned();
        let mut hinits: Vec<hir::FieldInit> = inits
            .iter()
            .enumerate()
            .map(|(i, fi)| {
                let expected = struct_fields.as_ref().and_then(|fields| {
                    if let Some(fname) = fi.name.as_ref() {
                        // Named field: look up by name
                        fields
                            .iter()
                            .find(|(n, _)| n == fname)
                            .map(|(_, ty)| ty.clone())
                    } else {
                        // Positional field: look up by index
                        fields.get(i).map(|(_, ty)| ty.clone())
                    }
                });
                Ok(hir::FieldInit {
                    name: fi.name.clone(),
                    value: self.lower_expr_expected(&fi.value, expected.as_ref())?,
                })
            })
            .collect::<Result<_, String>>()?;

        let arg_tys: Vec<Type> = hinits.iter().map(|fi| fi.value.ty.clone()).collect();
        if let Ok(Some(mangled)) = self.try_monomorphize_generic_variant(name, &arg_tys) {
            let (_, tag) = self
                .variant_tags
                .get(name)
                .cloned()
                .unwrap_or((mangled.clone(), 0));
            return Ok(hir::Expr {
                kind: hir::ExprKind::VariantCtor(mangled.clone(), name.to_string(), tag, hinits),
                ty: Type::Enum(mangled),
                span,
            });
        }

        if let Some(fields) = self.structs.get(name).cloned() {
            for (i, fi) in hinits.iter_mut().enumerate() {
                let declared_ty = if let Some(fname) = &fi.name {
                    // Named field: find by name
                    fields.iter().find(|(n, _)| n == fname).map(|(_, ty)| ty)
                } else {
                    // Positional field: find by index
                    fields.get(i).map(|(_, ty)| ty)
                };
                if let Some(declared_ty) = declared_ty {
                    let _ = self.infer_ctx.unify_at(
                        declared_ty,
                        &fi.value.ty,
                        span,
                        "struct literal field",
                    );
                    let taken = std::mem::replace(
                        &mut fi.value,
                        hir::Expr {
                            kind: hir::ExprKind::Void,
                            ty: Type::Void,
                            span,
                        },
                    );
                    fi.value = self.maybe_coerce_to(taken, declared_ty);
                }
            }
        }

        Ok(hir::Expr {
            kind: hir::ExprKind::Struct(name.to_string(), hinits),
            ty: Type::Struct(name.to_string()),
            span,
        })
    }

    fn lower_lambda_with_expected(
        &mut self,
        params: &[ast::Param],
        ret: &Option<Type>,
        body: &ast::Block,
        span: Span,
        expected: Option<&Type>,
    ) -> Result<hir::Expr, String> {
        let (expected_ptys, expected_ret) = match expected {
            Some(Type::Fn(ptys, ret)) => (Some(ptys.as_slice()), Some(ret.as_ref())),
            _ => (None, None),
        };

        self.push_scope();
        let mut hparams = Vec::new();
        let mut ptys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let pid = self.fresh_id();
            let ty = p.ty.clone().unwrap_or_else(|| {
                expected_ptys
                    .and_then(|ep| ep.get(i))
                    .cloned()
                    .unwrap_or_else(|| self.infer_ctx.fresh_var())
            });
            ptys.push(ty.clone());
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name,
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                    scheme: None,
                },
            );
            hparams.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                span: p.span,
            });
        }

        let ret_ty = ret.clone().unwrap_or_else(|| {
            if let Some(eret) = expected_ret {
                eret.clone()
            } else {
                // Phase 1.2: Use fresh TypeVar instead of AST heuristic
                self.infer_ctx.fresh_var()
            }
        });

        let hbody = self.lower_block_no_scope(body, &ret_ty)?;
        self.pop_scope();

        // Always unify ret_ty with the body's tail expression type.
        // This ensures that:
        // (a) fresh ret TypeVars (no annotation, no expected) get resolved
        // (b) expected return TypeVars (from scheme instantiation) get unified
        //     with the body's actual return type, solving them transitively
        // (c) explicit return annotations get validated against the body
        if let Some(hir::Stmt::Expr(e)) = hbody.last() {
            if e.ty != Type::Void {
                let _ = self.infer_ctx.unify(&ret_ty, &e.ty);
            }
        }

        let final_ret = if ret.is_some() || expected_ret.is_some() {
            ret_ty
        } else {
            match hbody.last() {
                Some(hir::Stmt::Expr(e)) if e.ty != Type::Void => e.ty.clone(),
                _ => {
                    let _ = self.infer_ctx.unify(&ret_ty, &Type::Void);
                    Type::Void
                }
            }
        };

        Ok(hir::Expr {
            kind: hir::ExprKind::Lambda(hparams, hbody),
            ty: Type::Fn(ptys, Box::new(final_ret)),
            span,
        })
    }

    /// Build a type_map from a generic function's params and resolved arg types.
    /// Ensures the generic fn is registered in `self.generic_fns`.
    pub(crate) fn maybe_coerce_to(&self, expr: hir::Expr, target: &Type) -> hir::Expr {
        if &expr.ty == target {
            return expr;
        }
        if let Some(coercion) = Self::needs_int_coercion(&expr.ty, target) {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(Box::new(expr), coercion),
                ty: target.clone(),
                span,
            };
        }
        if expr.ty.is_int() && target.is_float() {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(
                    Box::new(expr),
                    CoercionKind::IntToFloat { signed: true },
                ),
                ty: target.clone(),
                span,
            };
        }
        if expr.ty.is_float() && target.is_int() {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(
                    Box::new(expr),
                    CoercionKind::FloatToInt {
                        signed: target.is_signed(),
                    },
                ),
                ty: target.clone(),
                span,
            };
        }
        if expr.ty.is_float() && target.is_float() && expr.ty.bits() != target.bits() {
            let span = expr.span;
            let coercion = if expr.ty.bits() < target.bits() {
                CoercionKind::FloatWiden
            } else {
                CoercionKind::FloatNarrow
            };
            return hir::Expr {
                kind: hir::ExprKind::Coerce(Box::new(expr), coercion),
                ty: target.clone(),
                span,
            };
        }
        if expr.ty == Type::Bool && target.is_int() {
            let span = expr.span;
            return hir::Expr {
                kind: hir::ExprKind::Coerce(Box::new(expr), CoercionKind::BoolToInt),
                ty: target.clone(),
                span,
            };
        }
        if let Type::DynTrait(trait_name) = &target {
            if let Type::Struct(type_name) = &expr.ty {
                let tn = type_name.clone();
                let trn = trait_name.clone();
                let span = expr.span;
                return hir::Expr {
                    kind: hir::ExprKind::DynCoerce(Box::new(expr), tn, trn),
                    ty: target.clone(),
                    span,
                };
            }
        }
        expr
    }
}
