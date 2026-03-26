use std::collections::HashMap;
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
                let ty = expected.cloned().unwrap_or_else(|| self.infer_ctx.fresh_var());
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
                    let ty = match scheme_clone {
                        Some(ref scheme) if scheme.is_poly() => {
                            self.infer_ctx.instantiate(scheme)
                        }
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
                    let fn_ty = if let Some((ref q, ref sp, ref sr)) = self.fn_schemes.get(name).cloned() {
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
                let _ = self.infer_ctx.unify_at(&hl.ty, &hr.ty, *span, "binary operands");

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
                    let _ = self.infer_ctx.unify_at(exp, &result.ty, *span, "call result");
                }
                Ok(result)
            }

            ast::Expr::Method(obj, method, args, span) => {
                let result = self.lower_method_call(obj, method, args, *span)?;
                if let Some(exp) = expected {
                    let _ = self.infer_ctx.unify_at(exp, &result.ty, *span, "method call result");
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
                } else if matches!(&resolved_ty, Type::Vec(_)) && (field == "length" || field == "len") {
                    (Type::I64, 0)
                } else if matches!(&resolved_ty, Type::Map(_, _)) && (field == "length" || field == "len") {
                    (Type::I64, 0)
                } else if let Type::Tuple(ref tys) = resolved_ty {
                    // Tuple field access: .0, .1, etc.
                    if let Ok(idx) = field.parse::<usize>() {
                        if idx < tys.len() {
                            (tys[idx].clone(), idx)
                        } else {
                            return Err(format!(
                                "line {}:{}: tuple index {} out of range (tuple has {} elements)",
                                span.line, span.col, idx, tys.len()
                            ));
                        }
                    } else {
                        (self.infer_ctx.fresh_var(), 0)
                    }
                } else if matches!(resolved_ty, Type::TypeVar(_)) {
                    // Row polymorphism: try to infer struct type from field access.
                    // Search all known structs for one that has this field.
                    let candidates: Vec<(String, Type, usize)> = self.structs.iter()
                        .filter_map(|(sname, fields)| {
                            fields.iter().enumerate()
                                .find(|(_, (fname, _))| fname == field)
                                .map(|(idx, (_, fty))| (sname.clone(), fty.clone(), idx))
                        })
                        .collect();

                    if candidates.len() == 1 {
                        // Unique match — constrain the TypeVar to this struct
                        let (sname, fty, idx) = &candidates[0];
                        let struct_ty = Type::Struct(sname.clone());
                        let _ = self.infer_ctx.unify_at(
                            &resolved_ty, &struct_ty, *span,
                            "field access implies struct type",
                        );
                        let fty = self.infer_ctx.shallow_resolve(fty);
                        (fty, *idx)
                    } else {
                        // Ambiguous or no match — defer
                        let fty = self.infer_ctx.fresh_var();
                        self.deferred_fields.push(super::DeferredField {
                            receiver_ty: resolved_ty.clone(),
                            field_name: field.clone(),
                            field_ty: fty.clone(),
                            span: *span,
                        });
                        (fty, 0)
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
                    Type::Tuple(tys) => tys.first().cloned().unwrap_or_else(|| self.infer_ctx.fresh_var()),
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
                let result_ty = expected.cloned().unwrap_or_else(|| self.infer_ctx.fresh_var());
                let hi = self.lower_if(i, &result_ty)?;
                let ty = match hi.then.last() {
                    Some(hir::Stmt::Expr(e)) => e.ty.clone(),
                    _ => Type::Void,
                };
                if let Some(ref els) = hi.els {
                    if let Some(hir::Stmt::Expr(e)) = els.last() {
                        let _ =
                            self.infer_ctx
                                .unify_at(&ty, &e.ty, i.span, "if-expression branches");
                    }
                }
                for (_, branch) in &hi.elifs {
                    if let Some(hir::Stmt::Expr(e)) = branch.last() {
                        let _ = self.infer_ctx.unify_at(&ty, &e.ty, i.span, "elif branch");
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
                ty: expected.cloned().unwrap_or_else(|| self.infer_ctx.fresh_var()),
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
                        let _ = self.infer_ctx.unify_at(&resolved, &ptr_ty, *span, "dereference");
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
                let hcond = cond.as_ref().map(|m| self.lower_expr_expected(m, Some(&Type::Bool))).transpose()?;
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
                let elem_ty = match &hch.ty {
                    Type::Channel(t) => (**t).clone(),
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
                let elem_ty = match &hch.ty {
                    Type::Channel(t) => (**t).clone(),
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
                    let elem_ty = match &hch.ty {
                        Type::Channel(t) => (**t).clone(),
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

    fn lower_call(
        &mut self,
        callee: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        if let ast::Expr::Ident(name, _) = callee {
            match name.as_str() {
                "assert" => {
                    if args.is_empty() {
                        return Err("assert requires a condition".into());
                    }
                    let hcond = self.lower_expr(&args[0])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Assert, vec![hcond]),
                        ty: Type::Void,
                        span,
                    });
                }
                "log" => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Log, hargs),
                        ty: Type::Void,
                        span,
                    });
                }
                "to_string" => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::ToString, hargs),
                        ty: Type::String,
                        span,
                    });
                }
                "rc" if args.len() == 1 => {
                    let harg = self.lower_expr(&args[0])?;
                    let inner_ty = harg.ty.clone();
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcAlloc, vec![harg]),
                        ty: Type::Rc(Box::new(inner_ty)),
                        span,
                    });
                }
                "rc_retain" => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcRetain, hargs),
                        ty: Type::Void,
                        span,
                    });
                }
                "rc_release" => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::RcRelease, hargs),
                        ty: Type::Void,
                        span,
                    });
                }
                "weak" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let harg = self.lower_expr(&args[0])?;
                    let inner_ty = match &harg.ty {
                        Type::Rc(inner) => inner.as_ref().clone(),
                        _ => return Err(format!("weak() requires an rc value, got {}", harg.ty)),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::WeakDowngrade, vec![harg]),
                        ty: Type::Weak(Box::new(inner_ty)),
                        span,
                    });
                }
                "weak_upgrade" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let harg = self.lower_expr(&args[0])?;
                    let inner_ty = match &harg.ty {
                        Type::Weak(inner) => inner.as_ref().clone(),
                        _ => {
                            return Err(format!(
                                "weak_upgrade() requires a weak value, got {}",
                                harg.ty
                            ));
                        }
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::WeakUpgrade, vec![harg]),
                        ty: Type::Rc(Box::new(inner_ty)),
                        span,
                    });
                }
                "volatile_load" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let harg = self.lower_expr(&args[0])?;
                    let inner_ty = match &harg.ty {
                        Type::Ptr(inner) => inner.as_ref().clone(),
                        _ => {
                            return Err(format!(
                                "volatile_load() requires a pointer, got {}",
                                harg.ty
                            ));
                        }
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::VolatileLoad, vec![harg]),
                        ty: inner_ty,
                        span,
                    });
                }
                "volatile_store" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hptr = self.lower_expr(&args[0])?;
                    let hval = self.lower_expr(&args[1])?;
                    if !matches!(hptr.ty, Type::Ptr(_)) {
                        return Err(format!(
                            "volatile_store() first arg must be a pointer, got {}",
                            hptr.ty
                        ));
                    }
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(
                            hir::BuiltinFn::VolatileStore,
                            vec![hptr, hval],
                        ),
                        ty: Type::Void,
                        span,
                    });
                }
                "wrapping_add" | "wrapping_sub" | "wrapping_mul"
                    if args.len() == 2 && !self.fns.contains_key(name) =>
                {
                    let lhs = self.lower_expr(&args[0])?;
                    let rhs = self.lower_expr(&args[1])?;
                    let ty = lhs.ty.clone();
                    let builtin = match name.as_str() {
                        "wrapping_add" => hir::BuiltinFn::WrappingAdd,
                        "wrapping_sub" => hir::BuiltinFn::WrappingSub,
                        "wrapping_mul" => hir::BuiltinFn::WrappingMul,
                        _ => unreachable!(),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                        ty,
                        span,
                    });
                }
                "saturating_add" | "saturating_sub" | "saturating_mul"
                    if args.len() == 2 && !self.fns.contains_key(name) =>
                {
                    let lhs = self.lower_expr(&args[0])?;
                    let rhs = self.lower_expr(&args[1])?;
                    let ty = lhs.ty.clone();
                    let builtin = match name.as_str() {
                        "saturating_add" => hir::BuiltinFn::SaturatingAdd,
                        "saturating_sub" => hir::BuiltinFn::SaturatingSub,
                        "saturating_mul" => hir::BuiltinFn::SaturatingMul,
                        _ => unreachable!(),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                        ty,
                        span,
                    });
                }
                "checked_add" | "checked_sub" | "checked_mul"
                    if args.len() == 2 && !self.fns.contains_key(name) =>
                {
                    let lhs = self.lower_expr(&args[0])?;
                    let rhs = self.lower_expr(&args[1])?;
                    let ty = lhs.ty.clone();
                    let builtin = match name.as_str() {
                        "checked_add" => hir::BuiltinFn::CheckedAdd,
                        "checked_sub" => hir::BuiltinFn::CheckedSub,
                        "checked_mul" => hir::BuiltinFn::CheckedMul,
                        _ => unreachable!(),
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(builtin, vec![lhs, rhs]),
                        ty: Type::Tuple(vec![ty, Type::Bool]),
                        span,
                    });
                }
                "signal_handle" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hsig = self.lower_expr(&args[0])?;
                    let hhandler = self.lower_expr(&args[1])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(
                            hir::BuiltinFn::SignalHandle,
                            vec![hsig, hhandler],
                        ),
                        ty: Type::Void,
                        span,
                    });
                }
                "signal_raise" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hsig = self.lower_expr(&args[0])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::SignalRaise, vec![hsig]),
                        ty: Type::I32,
                        span,
                    });
                }
                "signal_ignore" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hsig = self.lower_expr(&args[0])?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::SignalIgnore, vec![hsig]),
                        ty: Type::Void,
                        span,
                    });
                }
                "popcount" | "clz" | "ctz" | "rotate_left" | "rotate_right" | "bswap" => {
                    let builtin = match name.as_str() {
                        "popcount" => hir::BuiltinFn::Popcount,
                        "clz" => hir::BuiltinFn::Clz,
                        "ctz" => hir::BuiltinFn::Ctz,
                        "rotate_left" => hir::BuiltinFn::RotateLeft,
                        "rotate_right" => hir::BuiltinFn::RotateRight,
                        "bswap" => hir::BuiltinFn::Bswap,
                        _ => unreachable!(),
                    };
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(builtin, hargs),
                        ty: Type::I64,
                        span,
                    });
                }
                "__string_from_raw" if args.len() == 3 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::StringFromRaw, hargs),
                        ty: Type::String,
                        span,
                    });
                }
                "__string_from_ptr" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::StringFromPtr, hargs),
                        ty: Type::String,
                        span,
                    });
                }
                "__get_args" if args.is_empty() && !self.fns.contains_key(name) => {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::GetArgs, vec![]),
                        ty: Type::Vec(Box::new(Type::String)),
                        span,
                    });
                }
                "__ln" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Ln, hargs),
                        ty: Type::F64,
                        span,
                    });
                }
                "__log2" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Log2, hargs),
                        ty: Type::F64,
                        span,
                    });
                }
                "__log10" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Log10, hargs),
                        ty: Type::F64,
                        span,
                    });
                }
                "__exp" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Exp, hargs),
                        ty: Type::F64,
                        span,
                    });
                }
                "__exp2" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Exp2, hargs),
                        ty: Type::F64,
                        span,
                    });
                }
                "__powf" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::PowF, hargs),
                        ty: Type::F64,
                        span,
                    });
                }
                "__copysign" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Copysign, hargs),
                        ty: Type::F64,
                        span,
                    });
                }
                "__fma" if args.len() == 3 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::Fma, hargs),
                        ty: Type::F64,
                        span,
                    });
                }
                "__fmt_float" if args.len() == 2 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::FmtFloat, hargs),
                        ty: Type::String,
                        span,
                    });
                }
                "__fmt_hex" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::FmtHex, hargs),
                        ty: Type::String,
                        span,
                    });
                }
                "__fmt_oct" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::FmtOct, hargs),
                        ty: Type::String,
                        span,
                    });
                }
                "__fmt_bin" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::FmtBin, hargs),
                        ty: Type::String,
                        span,
                    });
                }
                "__time_monotonic" if args.is_empty() && !self.fns.contains_key(name) => {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::TimeMonotonic, vec![]),
                        ty: Type::F64,
                        span,
                    });
                }
                "__sleep_ms" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::SleepMs, hargs),
                        ty: Type::Void,
                        span,
                    });
                }
                "__file_exists" if args.len() == 1 && !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Builtin(hir::BuiltinFn::FileExists, hargs),
                        ty: Type::Bool,
                        span,
                    });
                }
                "vec" if !self.fns.contains_key(name) => {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    let elem_ty = hargs
                        .first()
                        .map(|a| a.ty.clone())
                        .unwrap_or_else(|| self.infer_ctx.fresh_integer_var());
                    for a in hargs.iter().skip(1) {
                        let _ = self
                            .infer_ctx
                            .unify_at(&elem_ty, &a.ty, span, "vec element");
                    }
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::VecNew(hargs),
                        ty: Type::Vec(Box::new(elem_ty)),
                        span,
                    });
                }
                "map" if !self.fns.contains_key(name) => {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::MapNew,
                        ty: Type::Map(Box::new(Type::String), Box::new(self.infer_ctx.fresh_integer_var())),
                        span,
                    });
                }
                _ => {}
            }

            // ── Scheme-based polymorphic call resolution (HM generalization) ──
            // If the function has a generalized scheme in fn_schemes, instantiate
            // it with fresh TypeVars so each call site gets independent type
            // solutions. Then monomorphize for codegen with the resolved types.
            if let Some((ref quantified, ref scheme_params, ref scheme_ret)) = self.fn_schemes.get(name).cloned() {
                if !quantified.is_empty() {
                    let scheme = crate::types::Scheme {
                        quantified: quantified.clone(),
                        ty: Type::Fn(scheme_params.clone(), Box::new(scheme_ret.clone())),
                    };
                    let instantiated = self.infer_ctx.instantiate(&scheme);
                    let (inst_params, _inst_ret) = match instantiated {
                        Type::Fn(ps, r) => (ps, *r),
                        _ => unreachable!("scheme instantiation should produce Fn type"),
                    };

                    // Lower args with expected types from instantiation
                    let mut hargs: Vec<hir::Expr> = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        let expected = inst_params.get(i);
                        hargs.push(self.lower_expr_expected(arg, expected)?);
                    }

                    // Unify each arg with the instantiated param type
                    for (i, ha) in hargs.iter().enumerate() {
                        if let Some(pt) = inst_params.get(i) {
                            let _ = self.infer_ctx.unify_at(pt, &ha.ty, span, "function argument");
                        }
                    }

                    // Resolve instantiated types to concrete types for monomorphization.
                    // Temporarily disable strict mode to avoid false positives on
                    // TypeVars that are properly being defaulted from call-site arguments
                    // (e.g., integer literal `40` → Integer constraint → I64 default).
                    let was_strict = self.infer_ctx.is_strict();
                    self.infer_ctx.set_strict(false);
                    let arg_tys: Vec<Type> = inst_params.iter()
                        .map(|t| self.infer_ctx.resolve(t))
                        .collect();
                    self.infer_ctx.set_strict(was_strict);

                    // Build type_map for monomorphization
                    let inf_fn = self.inferable_fns.get(name).cloned()
                        .expect("fn_schemes should have corresponding inferable_fn");
                    let normalized = Self::normalize_inferable_fn(&inf_fn);
                    if !self.generic_fns.contains_key(name) {
                        self.generic_fns.insert(name.to_string(), normalized.clone());
                    }
                    let mut type_map = HashMap::new();
                    for (i, p) in normalized.params.iter().enumerate() {
                        if let Some(Type::Param(tp)) = &p.ty {
                            if i < arg_tys.len() {
                                type_map.insert(tp.clone(), arg_tys[i].clone());
                            }
                        }
                    }
                    for tp in &normalized.type_params {
                        type_map.entry(tp.clone()).or_insert(Type::I64);
                    }

                    let mangled = self.monomorphize_fn(name, &type_map)?;
                    let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();

                    // Coerce args if needed
                    let mono_param_tys = self.fns.get(&mangled).map(|(_, pts, _)| pts.clone()).unwrap_or_default();
                    for (i, ha) in hargs.iter_mut().enumerate() {
                        if let Some(pt) = mono_param_tys.get(i) {
                            let taken = std::mem::replace(
                                ha,
                                hir::Expr { kind: hir::ExprKind::Int(0), ty: Type::I64, span },
                            );
                            *ha = self.maybe_coerce_to(taken, pt);
                        }
                    }

                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Call(id, mangled, hargs),
                        ty: ret,
                        span,
                    });
                }
            }

            if let Some(gf) = self.generic_fns.get(name).cloned() {
                // Explicitly generic functions (with Type::Param) — monomorphize directly
                // Skip inferable fns that have poly schemes (handled above by scheme path)
                // Skip inferable fns that don't have schemes YET (self-recursive lowering)
                // Skip inferable fns with non-poly schemes (monomorphic) — they should
                // fall through to self.fns path for proper arg/param unification
                let has_poly_scheme = self.fn_schemes.get(name).map_or(false, |s| !s.0.is_empty());
                let is_inferable = self.inferable_fns.contains_key(name);
                let is_inferable_without_scheme = is_inferable && !self.fn_schemes.contains_key(name);
                if !has_poly_scheme && !is_inferable_without_scheme && !is_inferable {
                    let hargs: Vec<hir::Expr> = args
                        .iter()
                        .map(|e| self.lower_expr(e))
                        .collect::<Result<_, _>>()?;
                    let arg_tys: Vec<Type> = hargs.iter().map(|e| self.infer_ctx.resolve(&e.ty)).collect();
                    let mut type_map = HashMap::new();
                    for (i, p) in gf.params.iter().enumerate() {
                        if let Some(Type::Param(tp)) = &p.ty {
                            if i < arg_tys.len() {
                                type_map.insert(tp.clone(), arg_tys[i].clone());
                            }
                        }
                    }
                    for tp in &gf.type_params {
                        type_map.entry(tp.clone()).or_insert(Type::I64);
                    }
                    let mangled = self.monomorphize_fn(name, &type_map)?;
                    let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Call(id, mangled, hargs),
                        ty: ret,
                        span,
                    });
                }
            }

            // Auto-monomorphization fallback for inferable functions:
            // When a function with unannotated params has its TypeVars already solved
            // to concrete types that conflict with the current call's arg types,
            // fall back to monomorphization.
            // Skip during initial body lowering (before scheme is built) to preserve
            // TypeVar polymorphism for generalization.
            if let Some(inf_fn) = self.inferable_fns.get(name).cloned() {
                // Skip if already handled by scheme-based path above
                // Also skip if scheme hasn't been built yet (self-recursive lowering)
                if !self.fn_schemes.get(name).map_or(false, |s| !s.0.is_empty())
                    && self.fn_schemes.contains_key(name) {
                    if let Some((_, param_tys, _)) = self.fns.get(name).cloned() {
                        let hargs: Vec<hir::Expr> = args
                            .iter()
                            .map(|e| self.lower_expr(e))
                            .collect::<Result<_, _>>()?;
                        // Use shallow_resolve (not resolve) to avoid triggering
                        // strict errors on TypeVars that will be resolved via
                        // unification in the needs_mono check below
                        let arg_tys: Vec<Type> = hargs.iter().map(|e| self.infer_ctx.shallow_resolve(&e.ty)).collect();
                        let resolved_params: Vec<Type> = param_tys.iter().map(|t| self.infer_ctx.shallow_resolve(t)).collect();
                        let needs_mono = resolved_params.iter().zip(arg_tys.iter()).any(|(pt, at)| {
                            !matches!(pt, Type::TypeVar(_)) && pt != at && self.infer_ctx.unify(pt, at).is_err()
                        });
                        if needs_mono {
                            let normalized = Self::normalize_inferable_fn(&inf_fn);
                            if !self.generic_fns.contains_key(name) {
                                self.generic_fns.insert(name.to_string(), normalized.clone());
                            }
                            let mut type_map = HashMap::new();
                            for (i, p) in normalized.params.iter().enumerate() {
                                if let Some(Type::Param(tp)) = &p.ty {
                                    if i < arg_tys.len() {
                                        type_map.insert(tp.clone(), arg_tys[i].clone());
                                    }
                                }
                            }
                            for tp in &normalized.type_params {
                                type_map.entry(tp.clone()).or_insert(Type::I64);
                            }
                            let mangled = self.monomorphize_fn(name, &type_map)?;
                            let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
                            return Ok(hir::Expr {
                                kind: hir::ExprKind::Call(id, mangled, hargs),
                                ty: ret,
                                span,
                            });
                        }
                    }
                }
            }

            if let Some((id, param_tys, ret)) = self.fns.get(name).cloned() {
                let mut hargs: Vec<hir::Expr> = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let expected = param_tys.get(i);
                    hargs.push(self.lower_expr_expected(arg, expected)?);
                }
                for (i, ha) in hargs.iter().enumerate() {
                    if let Some(pt) = param_tys.get(i) {
                        let _ = self
                            .infer_ctx
                            .unify_at(pt, &ha.ty, span, "function argument");
                    }
                }
                for (i, ha) in hargs.iter_mut().enumerate() {
                    if let Some(pt) = param_tys.get(i) {
                        let taken = std::mem::replace(
                            ha,
                            hir::Expr {
                                kind: hir::ExprKind::Int(0),
                                ty: Type::I64,
                                span,
                            },
                        );
                        *ha = self.maybe_coerce_to(taken, pt);
                    }
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, name.clone(), hargs),
                    ty: ret,
                    span,
                });
            }

            if let Some(v) = self.find_var(name).cloned() {
                let resolved_ty = self.infer_ctx.shallow_resolve(&v.ty);
                if let Type::Fn(ptys, ret) = &resolved_ty {
                    let ret = *ret.clone();
                    let ptys = ptys.clone();
                    let fn_expr = hir::Expr {
                        kind: hir::ExprKind::Var(v.def_id, name.clone()),
                        ty: resolved_ty.clone(),
                        span,
                    };
                    let mut hargs = Vec::new();
                    for (i, arg) in args.iter().enumerate() {
                        let expected = ptys.get(i);
                        hargs.push(self.lower_expr_expected(arg, expected)?);
                    }
                    for (i, ha) in hargs.iter().enumerate() {
                        if let Some(pt) = ptys.get(i) {
                            let _ =
                                self.infer_ctx
                                    .unify_at(pt, &ha.ty, span, "indirect call argument");
                        }
                    }
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::IndirectCall(Box::new(fn_expr), hargs),
                        ty: ret,
                        span,
                    });
                }
                // Phase 6: Higher-order inference — when a variable with TypeVar
                // type is called, unify it with Fn(arg_tys) -> fresh_ret
                if matches!(resolved_ty, Type::TypeVar(_) | Type::Param(_)) {
                    let mut hargs = Vec::new();
                    for arg in args.iter() {
                        hargs.push(self.lower_expr(arg)?);
                    }
                    let arg_tys: Vec<Type> = hargs.iter().map(|a| a.ty.clone()).collect();
                    let ret = self.infer_ctx.fresh_var();
                    let fn_ty = Type::Fn(arg_tys, Box::new(ret.clone()));
                    let _ = self.infer_ctx.unify_at(&v.ty, &fn_ty, span, "higher-order call");
                    let fn_expr = hir::Expr {
                        kind: hir::ExprKind::Var(v.def_id, name.clone()),
                        ty: fn_ty,
                        span,
                    };
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::IndirectCall(Box::new(fn_expr), hargs),
                        ty: ret,
                        span,
                    });
                }
            }

            let _hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            return Err(format!("undefined function: '{name}'"));
        }

        let hcallee = self.lower_expr(callee)?;
        let callee_resolved = self.infer_ctx.shallow_resolve(&hcallee.ty);
        let (ptys, ret) = match &callee_resolved {
            Type::Fn(ptys, ret) => (ptys.clone(), *ret.clone()),
            _ => {
                // Phase 6: Higher-order inference — construct Fn type from args
                // and unify with the callee's TypeVar
                let mut hargs: Vec<hir::Expr> = Vec::new();
                for arg in args.iter() {
                    hargs.push(self.lower_expr(arg)?);
                }
                let arg_tys: Vec<Type> = hargs.iter().map(|a| a.ty.clone()).collect();
                let ret = self.infer_ctx.fresh_var();
                let fn_ty = Type::Fn(arg_tys, Box::new(ret.clone()));
                let _ = self.infer_ctx.unify_at(&hcallee.ty, &fn_ty, span, "higher-order call");
                return Ok(hir::Expr {
                    kind: hir::ExprKind::IndirectCall(Box::new(hir::Expr {
                        ty: fn_ty,
                        ..hcallee
                    }), hargs),
                    ty: ret,
                    span,
                });
            }
        };
        let mut hargs: Vec<hir::Expr> = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let expected = ptys.get(i);
            hargs.push(self.lower_expr_expected(arg, expected)?);
        }
        for (i, ha) in hargs.iter().enumerate() {
            if let Some(pt) = ptys.get(i) {
                let _ = self
                    .infer_ctx
                    .unify_at(pt, &ha.ty, span, "indirect call argument");
            }
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::IndirectCall(Box::new(hcallee), hargs),
            ty: ret,
            span,
        })
    }

    fn lower_method_call(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        // Phase 1.2: Lower object FIRST, then dispatch on its resolved HIR type.
        let hobj = self.lower_expr(obj)?;
        let obj_ty = self.infer_ctx.shallow_resolve(&hobj.ty);

        if matches!(obj_ty, Type::String) {
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "contains" | "starts_with" | "ends_with" => Type::Bool,
                "char_at" | "len" | "find" => Type::I64,
                "slice" | "trim" | "trim_left" | "trim_right" | "replace" | "to_upper"
                | "to_lower" => Type::String,
                "split" => Type::Vec(Box::new(Type::String)),
                _ => Type::I64,
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::StringMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Vec(ref elem_ty) = obj_ty {
            let expected_arg_tys: Vec<Option<&Type>> = match method {
                "push" => vec![Some(elem_ty.as_ref())],
                "set" => vec![Some(&Type::I64), Some(elem_ty.as_ref())],
                "get" | "remove" => vec![Some(&Type::I64)],
                _ => vec![],
            };
            let hargs: Vec<hir::Expr> = args
                .iter()
                .enumerate()
                .map(|(i, e)| self.lower_expr_expected(e, expected_arg_tys.get(i).copied().flatten()))
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "push" | "clear" => Type::Void,
                "pop" | "get" | "remove" => *elem_ty.clone(),
                "len" => Type::I64,
                "set" => Type::Void,
                _ => return Err(format!("no method '{method}' on Vec")),
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::VecMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Map(ref key_ty, ref val_ty) = obj_ty {
            let expected_arg_tys: Vec<Option<&Type>> = match method {
                "set" => vec![Some(key_ty.as_ref()), Some(val_ty.as_ref())],
                "get" | "has" | "remove" => vec![Some(key_ty.as_ref())],
                _ => vec![],
            };
            let hargs: Vec<hir::Expr> = args
                .iter()
                .enumerate()
                .map(|(i, e)| self.lower_expr_expected(e, expected_arg_tys.get(i).copied().flatten()))
                .collect::<Result<_, _>>()?;
            let ret_ty = match method {
                "set" | "remove" | "clear" => Type::Void,
                "get" => *val_ty.clone(),
                "has" => Type::Bool,
                "len" => Type::I64,
                "keys" => Type::Vec(key_ty.clone()),
                "values" => Type::Vec(val_ty.clone()),
                _ => return Err(format!("no method '{method}' on Map")),
            };
            return Ok(hir::Expr {
                kind: hir::ExprKind::MapMethod(Box::new(hobj), method.to_string(), hargs),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Coroutine(ref yield_ty) = obj_ty {
            if method == "next" {
                return Ok(hir::Expr {
                    kind: hir::ExprKind::CoroutineNext(Box::new(hobj)),
                    ty: *yield_ty.clone(),
                    span,
                });
            }
            return Err(format!("no method '{method}' on Coroutine"));
        }

        if let Type::DynTrait(ref trait_name) = obj_ty {
            let hargs: Vec<hir::Expr> = args
                .iter()
                .map(|e| self.lower_expr(e))
                .collect::<Result<_, _>>()?;
            let ret_ty = self.infer_dyn_method_ret(trait_name, method);
            return Ok(hir::Expr {
                kind: hir::ExprKind::DynDispatch(
                    Box::new(hobj),
                    trait_name.clone(),
                    method.to_string(),
                    hargs,
                ),
                ty: ret_ty,
                span,
            });
        }

        if let Type::Struct(ref type_name) = obj_ty {
            let method_name = format!("{type_name}_{method}");
            if let Some((_, param_tys, ret)) = self.fns.get(&method_name).cloned() {
                // param_tys[0] is self, actual args start at [1]
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let expected = param_tys.get(i + 1);
                        self.lower_expr_expected(e, expected)
                    })
                    .collect::<Result<_, _>>()?;
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Method(
                        Box::new(hobj),
                        method_name,
                        method.to_string(),
                        hargs,
                    ),
                    ty: ret,
                    span,
                });
            }
        }

        // Fallback: if receiver is a TypeVar, try row polymorphism.
        // Search for struct methods matching `_method` suffix to infer struct type.
        if matches!(obj_ty, Type::TypeVar(_)) {
            let suffix = format!("_{method}");
            let mut candidates: Vec<(String, Vec<Type>, Type)> = self.fns.iter()
                .filter(|(name, _)| name.ends_with(&suffix))
                .map(|(name, (_, ptys, ret))| {
                    let type_name = name[..name.len() - suffix.len()].to_string();
                    (type_name, ptys.clone(), ret.clone())
                })
                .filter(|(type_name, _, _)| self.structs.contains_key(type_name))
                .collect();

            // Phase 3A: When multiple candidates, narrow using trait information.
            // Find traits that define this method, then keep only candidates whose
            // type implements at least one such trait.
            if candidates.len() > 1 {
                let defining_traits: Vec<&String> = self.traits.iter()
                    .filter(|(_, sigs)| sigs.iter().any(|s| s.name == method))
                    .map(|(tname, _)| tname)
                    .collect();
                if !defining_traits.is_empty() {
                    let narrowed: Vec<(String, Vec<Type>, Type)> = candidates.iter()
                        .filter(|(type_name, _, _)| {
                            self.trait_impls.get(type_name).map_or(false, |impls| {
                                impls.iter().any(|i| defining_traits.contains(&i))
                            })
                        })
                        .cloned()
                        .collect();
                    if !narrowed.is_empty() {
                        candidates = narrowed;
                    }
                }
            }

            if candidates.len() == 1 {
                let (type_name, param_tys, ret) = &candidates[0];
                let struct_ty = Type::Struct(type_name.clone());
                let _ = self.infer_ctx.unify_at(
                    &obj_ty, &struct_ty, span,
                    "method call implies struct type",
                );
                let method_name = format!("{}_{}", type_name, method);
                let hargs: Vec<hir::Expr> = args
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        let expected = param_tys.get(i + 1);
                        self.lower_expr_expected(e, expected)
                    })
                    .collect::<Result<_, _>>()?;
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Method(
                        Box::new(hobj),
                        method_name,
                        method.to_string(),
                        hargs,
                    ),
                    ty: ret.clone(),
                    span,
                });
            }
        }

        let hargs: Vec<hir::Expr> = args
            .iter()
            .map(|e| self.lower_expr(e))
            .collect::<Result<_, _>>()?;
        let ret_ty = self.infer_ctx.fresh_var();
        if matches!(obj_ty, Type::TypeVar(_)) {
            let arg_tys: Vec<Type> = hargs.iter().map(|a| a.ty.clone()).collect();

            // Trait-guided inference: if a trait defines this method,
            // use the return type from the trait signature as a constraint
            for (_, sigs) in &self.traits {
                for sig in sigs {
                    if sig.name == method {
                        if let Some(ref trait_ret) = sig._ret {
                            let _ = self.infer_ctx.unify_at(
                                &ret_ty, trait_ret, span,
                                "trait method return type",
                            );
                        }
                    }
                }
            }

            self.deferred_methods.push(super::DeferredMethod {
                receiver_ty: obj_ty.clone(),
                method: method.to_string(),
                arg_tys,
                ret_ty: ret_ty.clone(),
                span,
            });
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::StringMethod(Box::new(hobj), method.to_string(), hargs),
            ty: ret_ty,
            span,
        })
    }

    fn lower_struct_or_variant(
        &mut self,
        name: &str,
        inits: &[ast::FieldInit],
        span: Span,
    ) -> Result<hir::Expr, String> {
        if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
            // Look up variant field types from enum definition
            let variant_fields: Vec<Type> = self.enums.get(&enum_name)
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
        let hinits: Vec<hir::FieldInit> = inits
            .iter()
            .enumerate()
            .map(|(i, fi)| {
                let expected = struct_fields.as_ref().and_then(|fields| {
                    if let Some(fname) = fi.name.as_ref() {
                        // Named field: look up by name
                        fields.iter().find(|(n, _)| n == fname).map(|(_, ty)| ty.clone())
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
            for (i, fi) in hinits.iter().enumerate() {
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
                }
            }
        }

        Ok(hir::Expr {
            kind: hir::ExprKind::Struct(name.to_string(), hinits),
            ty: Type::Struct(name.to_string()),
            span,
        })
    }

    fn lower_pipe(
        &mut self,
        left: &ast::Expr,
        right: &ast::Expr,
        extra_args: &[ast::Expr],
        span: Span,
    ) -> Result<hir::Expr, String> {
        let hleft = self.lower_expr(left)?;
        if let ast::Expr::Ident(name, _) = right {
            if let Some(gf) = self.generic_fns.get(name).cloned() {
                let left_ty = hleft.ty.clone();
                let mut type_map = HashMap::new();
                if let Some(p) = gf.params.first() {
                    if let Some(Type::Param(tp)) = &p.ty {
                        type_map.insert(tp.clone(), left_ty);
                    }
                }
                for tp in &gf.type_params {
                    type_map.entry(tp.clone()).or_insert(Type::I64);
                }
                let mangled = self.monomorphize_fn(name, &type_map)?;
                let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
                let mut all_args = vec![hleft];
                for a in extra_args {
                    all_args.push(self.lower_expr(a)?);
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Call(id, mangled, all_args),
                    ty: ret,
                    span,
                });
            }
            if let Some((id, _, ret)) = self.fns.get(name).cloned() {
                let mut all_args = vec![hleft];
                for a in extra_args {
                    all_args.push(self.lower_expr(a)?);
                }
                return Ok(hir::Expr {
                    kind: hir::ExprKind::Pipe(
                        Box::new(all_args.remove(0)),
                        id,
                        name.clone(),
                        all_args,
                    ),
                    ty: ret,
                    span,
                });
            }
            let hright = self.lower_expr(right)?;
            let ret = match &hright.ty {
                Type::Fn(_, r) => *r.clone(),
                _ => self.infer_ctx.fresh_var(),
            };
            let mut all_args = vec![hleft];
            for a in extra_args {
                all_args.push(self.lower_expr(a)?);
            }
            return Ok(hir::Expr {
                kind: hir::ExprKind::IndirectCall(Box::new(hright), all_args),
                ty: ret,
                span,
            });
        }

        if let ast::Expr::Call(callee, call_args, _) = right {
            if let ast::Expr::Ident(name, _) = callee.as_ref() {
                let has_placeholder = call_args
                    .iter()
                    .any(|a| matches!(a, ast::Expr::Placeholder(_)));
                let mut all_args = Vec::new();
                if has_placeholder {
                    for a in call_args {
                        if matches!(a, ast::Expr::Placeholder(_)) {
                            all_args.push(hleft.clone());
                        } else {
                            all_args.push(self.lower_expr(a)?);
                        }
                    }
                } else {
                    all_args.push(hleft.clone());
                    for a in call_args {
                        all_args.push(self.lower_expr(a)?);
                    }
                }
                if let Some(gf) = self.generic_fns.get(name).cloned() {
                    let left_ty = all_args[0].ty.clone();
                    let mut type_map = HashMap::new();
                    if let Some(p) = gf.params.first() {
                        if let Some(Type::Param(tp)) = &p.ty {
                            type_map.insert(tp.clone(), left_ty);
                        }
                    }
                    for tp in &gf.type_params {
                        type_map.entry(tp.clone()).or_insert(Type::I64);
                    }
                    let mangled = self.monomorphize_fn(name, &type_map)?;
                    let (id, _, ret) = self.fns.get(&mangled).cloned().unwrap();
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Call(id, mangled, all_args),
                        ty: ret,
                        span,
                    });
                }
                if let Some((id, _, ret)) = self.fns.get(name).cloned() {
                    return Ok(hir::Expr {
                        kind: hir::ExprKind::Pipe(
                            Box::new(all_args.remove(0)),
                            id,
                            name.clone(),
                            all_args,
                        ),
                        ty: ret,
                        span,
                    });
                }
            }
        }

        let hright = self.lower_expr(right)?;
        let ret = match &hright.ty {
            Type::Fn(_, r) => *r.clone(),
            _ => self.infer_ctx.fresh_var(),
        };
        let mut all_args = vec![hleft];
        for a in extra_args {
            all_args.push(self.lower_expr(a)?);
        }
        Ok(hir::Expr {
            kind: hir::ExprKind::IndirectCall(Box::new(hright), all_args),
            ty: ret,
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
