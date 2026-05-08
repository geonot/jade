//! Extracted call-typing rules.

#![allow(unused_imports, unused_variables)]

use std::collections::HashMap;

use super::super::unify;
use super::super::{DeferredField, Typer, VarInfo};
use crate::ast::{self, Expr, Span};
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn dispatch_store_methods(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: crate::ast::Span,
    ) -> Result<Option<hir::Expr>, String> {
        // Store aggregation methods: store.avg(field), store.sum(field), etc.
        if let ast::Expr::Ident(name, _) = obj {
            if self.store_schemas.contains_key(&name.as_str()) {
                // @kv store methods: .set(), .get(), .del(), .has(), .incr(), .count()
                let is_kv = self
                    .store_decorators
                    .get(&name.as_str())
                    .map(|decs| decs.iter().any(|d| *d == crate::ast::StoreDecorator::Kv))
                    .unwrap_or(false);
                if is_kv {
                    match method {
                        "set" => {
                            if args.len() != 2 {
                                return Err("kv.set() requires 2 arguments (key, value)".into());
                            }
                            let key_expr =
                                self.lower_expr_expected(&args[0], Some(&Type::String))?;
                            let val_expr = self.lower_expr_expected(&args[1], Some(&Type::I64))?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::KvSet(
                                    name.clone(),
                                    Box::new(key_expr),
                                    Box::new(val_expr),
                                ),
                                ty: Type::Void,
                                span,
                            }));
                        }
                        "get" => {
                            if args.len() != 1 {
                                return Err("kv.get() requires 1 argument (key)".into());
                            }
                            let key_expr =
                                self.lower_expr_expected(&args[0], Some(&Type::String))?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::KvGet(name.clone(), Box::new(key_expr)),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        "has" => {
                            if args.len() != 1 {
                                return Err("kv.has() requires 1 argument (key)".into());
                            }
                            let key_expr =
                                self.lower_expr_expected(&args[0], Some(&Type::String))?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::KvHas(name.clone(), Box::new(key_expr)),
                                ty: Type::Bool,
                                span,
                            }));
                        }
                        "count" => {
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::KvCount(name.clone()),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        "del" => {
                            if args.len() != 1 {
                                return Err("kv.del() requires 1 argument (key)".into());
                            }
                            let key_expr =
                                self.lower_expr_expected(&args[0], Some(&Type::String))?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::KvDel(name.clone(), Box::new(key_expr)),
                                ty: Type::Void,
                                span,
                            }));
                        }
                        "incr" => {
                            let (key_expr, delta_expr) = if args.len() == 1 {
                                let k = self.lower_expr_expected(&args[0], Some(&Type::String))?;
                                let d = hir::Expr {
                                    kind: hir::ExprKind::Int(1),
                                    ty: Type::I64,
                                    span,
                                };
                                (k, d)
                            } else if args.len() == 2 {
                                let k = self.lower_expr_expected(&args[0], Some(&Type::String))?;
                                let d = self.lower_expr_expected(&args[1], Some(&Type::I64))?;
                                (k, d)
                            } else {
                                return Err(
                                    "kv.incr() requires 1-2 arguments (key [, delta])".into()
                                );
                            };
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::KvIncr(
                                    name.clone(),
                                    Box::new(key_expr),
                                    Box::new(delta_expr),
                                ),
                                ty: Type::Void,
                                span,
                            }));
                        }
                        "decr" => {
                            let key_expr = if args.len() >= 1 {
                                self.lower_expr_expected(&args[0], Some(&Type::String))?
                            } else {
                                return Err(
                                    "kv.decr() requires 1-2 arguments (key [, delta])".into()
                                );
                            };
                            let delta_expr = if args.len() == 2 {
                                // Negate the delta
                                let d = self.lower_expr_expected(&args[1], Some(&Type::I64))?;
                                hir::Expr {
                                    kind: hir::ExprKind::BinOp(
                                        Box::new(hir::Expr {
                                            kind: hir::ExprKind::Int(0),
                                            ty: Type::I64,
                                            span,
                                        }),
                                        crate::ast::BinOp::Sub,
                                        Box::new(d),
                                    ),
                                    ty: Type::I64,
                                    span,
                                }
                            } else {
                                hir::Expr {
                                    kind: hir::ExprKind::Int(-1),
                                    ty: Type::I64,
                                    span,
                                }
                            };
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::KvIncr(
                                    name.clone(),
                                    Box::new(key_expr),
                                    Box::new(delta_expr),
                                ),
                                ty: Type::Void,
                                span,
                            }));
                        }
                        _ => {
                            return Err(format!(
                                "@kv store supports .set(), .get(), .del(), .has(), .incr(), .decr(), .count(); got .{method}()"
                            ));
                        }
                    }
                }

                // @graph store methods
                let is_graph = self
                    .store_decorators
                    .get(&name.as_str())
                    .map(|decs| decs.iter().any(|d| *d == crate::ast::StoreDecorator::Graph))
                    .unwrap_or(false);
                if is_graph {
                    match method {
                        "from" => {
                            if args.len() != 1 {
                                return Err("graph.from() requires 1 argument (node)".into());
                            }
                            // .from(node) queries the first user field of the graph store
                            let schema = self.store_schemas.get(&name.as_str()).unwrap();
                            let builtin = [
                                "sid",
                                "uuid",
                                "hash",
                                "created",
                                "updated",
                                "deleted",
                                "__version",
                            ];
                            let user_fields: Vec<_> = schema
                                .iter()
                                .filter(|(n, _)| !builtin.iter().any(|b| *n == *b))
                                .collect();
                            let first_ty = user_fields
                                .first()
                                .map(|(_, t)| t.clone())
                                .unwrap_or(Type::I64);
                            let node_expr = self.lower_expr_expected(&args[0], Some(&first_ty))?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::GraphFrom(name.clone(), Box::new(node_expr)),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        "to" => {
                            if args.len() != 1 {
                                return Err("graph.to() requires 1 argument (node)".into());
                            }
                            // .to(node) queries the second user field of the graph store
                            let schema = self.store_schemas.get(&name.as_str()).unwrap();
                            let builtin = [
                                "sid",
                                "uuid",
                                "hash",
                                "created",
                                "updated",
                                "deleted",
                                "__version",
                            ];
                            let user_fields: Vec<_> = schema
                                .iter()
                                .filter(|(n, _)| !builtin.iter().any(|b| *n == *b))
                                .collect();
                            let second_ty = user_fields
                                .get(1)
                                .map(|(_, t)| t.clone())
                                .unwrap_or(Type::I64);
                            let node_expr = self.lower_expr_expected(&args[0], Some(&second_ty))?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::GraphTo(name.clone(), Box::new(node_expr)),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        _ => {} // fall through to aggregation methods
                    }
                }

                // @timeseries store methods
                let is_ts = self
                    .store_decorators
                    .get(&name.as_str())
                    .map(|decs| {
                        decs.iter()
                            .any(|d| matches!(d, crate::ast::StoreDecorator::TimeSeries(_)))
                    })
                    .unwrap_or(false);
                if is_ts {
                    match method {
                        "latest" => {
                            if !args.is_empty() {
                                return Err("timeseries.latest() takes no arguments".into());
                            }
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::TsLatest(name.clone()),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        _ => {} // fall through
                    }
                }

                // @vector store methods
                let vec_dims = self.store_decorators.get(&name.as_str()).and_then(|decs| {
                    decs.iter().find_map(|d| match d {
                        crate::ast::StoreDecorator::Vector(dims) => Some(*dims),
                        _ => None,
                    })
                });
                if let Some(_dims) = vec_dims {
                    match method {
                        "nearest" => {
                            if args.len() != 2 {
                                return Err(
                                    "vector.nearest() requires 2 arguments (query_array, k)".into(),
                                );
                            }
                            let query_expr = self.lower_expr(&args[0])?;
                            let k_expr = self.lower_expr(&args[1])?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::VecNearest(
                                    name.clone(),
                                    Box::new(query_expr),
                                    Box::new(k_expr),
                                ),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        "insert" => {
                            if args.len() != 1 {
                                return Err(
                                    "vector.insert() requires 1 argument (vector array)".into()
                                );
                            }
                            let vec_expr = self.lower_expr(&args[0])?;
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::VecInsert(name.clone(), Box::new(vec_expr)),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        "count" => {
                            if !args.is_empty() {
                                return Err("vector.count() takes no arguments".into());
                            }
                            return Ok(Some(hir::Expr {
                                kind: hir::ExprKind::VecCount(name.clone()),
                                ty: Type::I64,
                                span,
                            }));
                        }
                        _ => {} // fall through
                    }
                }

                // @bloom field methods: store.maybe(field, value)
                if method == "maybe" {
                    if args.len() != 2 {
                        return Err("maybe() requires 2 arguments (field_name, value)".into());
                    }
                    let field = match &args[0] {
                        ast::Expr::Ident(f, _) => f.clone(),
                        _ => return Err("maybe() first argument must be a field name".into()),
                    };
                    let val_expr = self.lower_expr(&args[1])?;
                    return Ok(Some(hir::Expr {
                        kind: hir::ExprKind::BloomTest(name.clone(), field, Box::new(val_expr)),
                        ty: Type::Bool,
                        span,
                    }));
                }

                // @search field methods: store.search(field, term)
                if method == "search" {
                    if args.len() != 2 {
                        return Err("search() requires 2 arguments (field_name, query)".into());
                    }
                    let field = match &args[0] {
                        ast::Expr::Ident(f, _) => f.clone(),
                        _ => return Err("search() first argument must be a field name".into()),
                    };
                    let query_expr = self.lower_expr(&args[1])?;
                    return Ok(Some(hir::Expr {
                        kind: hir::ExprKind::FtsSearch(name.clone(), field, Box::new(query_expr)),
                        ty: Type::I64,
                        span,
                    }));
                }
                if method == "search_count" {
                    if args.len() != 1 {
                        return Err("search_count() requires 1 argument (field_name)".into());
                    }
                    let field = match &args[0] {
                        ast::Expr::Ident(f, _) => f.clone(),
                        _ => return Err("search_count() argument must be a field name".into()),
                    };
                    return Ok(Some(hir::Expr {
                        kind: hir::ExprKind::FtsCount(name.clone(), field),
                        ty: Type::I64,
                        span,
                    }));
                }

                match method {
                    "sum" | "avg" | "min" | "max" => {
                        if args.len() != 1 {
                            return Err(format!("{method}() requires exactly 1 field argument"));
                        }
                        let field = match &args[0] {
                            ast::Expr::Ident(f, _) => f.clone(),
                            _ => return Err(format!("{method}() argument must be a field name")),
                        };
                        let kind = match method {
                            "sum" => hir::ExprKind::StoreSum(name.clone(), field.clone()),
                            "avg" => hir::ExprKind::StoreAvg(name.clone(), field.clone()),
                            "min" => hir::ExprKind::StoreMin(name.clone(), field.clone()),
                            "max" => hir::ExprKind::StoreMax(name.clone(), field.clone()),
                            _ => unreachable!(),
                        };
                        let field_ty = self.store_schemas.get(&name.as_str()).and_then(|schema| {
                            schema
                                .iter()
                                .find(|(n, _)| n == &field)
                                .map(|(_, t)| t.clone())
                        });
                        let ret_ty = if method == "avg" {
                            Type::F64
                        } else {
                            match field_ty {
                                Some(Type::F64) | Some(Type::F32) => Type::F64,
                                _ => Type::I64,
                            }
                        };
                        return Ok(Some(hir::Expr {
                            kind,
                            ty: ret_ty,
                            span,
                        }));
                    }
                    "distinct" => {
                        if args.len() != 1 {
                            return Err("distinct() requires exactly 1 field argument".into());
                        }
                        let field = match &args[0] {
                            ast::Expr::Ident(f, _) => f.clone(),
                            _ => return Err("distinct() argument must be a field name".into()),
                        };
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::StoreDistinct(name.clone(), field),
                            ty: Type::I64,
                            span,
                        }));
                    }
                    "count" => {
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::StoreCount(name.clone()),
                            ty: Type::I64,
                            span,
                        }));
                    }
                    "version_count" => {
                        if args.len() != 1 {
                            return Err("version_count() requires exactly 1 argument (sid)".into());
                        }
                        let sid_expr = self.lower_expr_expected(&args[0], Some(&Type::I64))?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::StoreVersionCount(
                                name.clone(),
                                Box::new(sid_expr),
                            ),
                            ty: Type::I64,
                            span,
                        }));
                    }
                    "history" => {
                        if args.len() != 1 {
                            return Err("history() requires exactly 1 argument (sid)".into());
                        }
                        let sid_expr = self.lower_expr_expected(&args[0], Some(&Type::I64))?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::StoreHistory(name.clone(), Box::new(sid_expr)),
                            ty: Type::I64, // returns count of versions written
                            span,
                        }));
                    }
                    "at_version" => {
                        if args.len() != 2 {
                            return Err("at_version() requires 2 arguments (sid, version)".into());
                        }
                        let sid_expr = self.lower_expr_expected(&args[0], Some(&Type::I64))?;
                        let ver_expr = self.lower_expr_expected(&args[1], Some(&Type::I64))?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::StoreAtVersion(
                                name.clone(),
                                Box::new(sid_expr),
                                Box::new(ver_expr),
                            ),
                            ty: Type::I64, // returns 1 if found, 0 if not
                            span,
                        }));
                    }
                    _ => {}
                }
            }
        }

        Ok(None)
    }
}
