use super::super::Typer;
use crate::ast;
use crate::hir;
use crate::types::Type;

const KV_BUILTIN_FIELDS: &[&str] = &[
    "sid",
    "uuid",
    "hash",
    "created",
    "updated",
    "deleted",
    "__version",
];

impl Typer {
    fn kv_key_val_types(&self, store: &str) -> (Type, Type) {
        let mut key_ty = Type::String;
        let mut val_ty = Type::I64;
        if let Some(schema) = self.store_schemas.get(store) {
            let user: Vec<&(crate::intern::Symbol, Type)> = schema
                .iter()
                .filter(|(n, _)| !KV_BUILTIN_FIELDS.contains(&&*n.as_str()))
                .collect();
            for (n, t) in &user {
                match &*n.as_str() {
                    "key" => key_ty = t.clone(),
                    "val" | "value" => val_ty = t.clone(),
                    _ => {}
                }
            }
        }
        (key_ty, val_ty)
    }

    fn kv_option_type(&mut self, val_ty: &Type) -> Result<crate::intern::Symbol, String> {
        let mut map = std::collections::HashMap::new();
        map.insert("T".into(), val_ty.clone());
        self.monomorphize_enum("Option", &map)
    }

    fn build_kv_get_option(
        &mut self,
        store: crate::intern::Symbol,
        key_expr: hir::Expr,
        val_ty: &Type,
        span: crate::ast::Span,
    ) -> Result<hir::Expr, String> {
        let opt_enum = self.kv_option_type(val_ty)?;
        let opt_ty = Type::Enum(opt_enum);
        let some_tag = self
            .enums
            .get(&opt_enum)
            .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Some"))
            .unwrap_or(0) as u32;
        let nothing_tag = self
            .enums
            .get(&opt_enum)
            .and_then(|vs| vs.iter().position(|(n, _)| n.as_str() == "Nothing"))
            .unwrap_or(1) as u32;

        let key_id = self.fresh_id();
        let key_bind = hir::Stmt::Bind(hir::Bind {
            def_id: key_id,
            name: "__kv_key".into(),
            value: key_expr,
            ty: Type::String,
            ownership: hir::Ownership::Owned,
            atomic: false,
            access_mod: None,
            span,
        });
        let key_ref = || hir::Expr {
            kind: hir::ExprKind::Var(key_id, "__kv_key".into()),
            ty: Type::String,
            span,
        };

        let has = hir::Expr {
            kind: hir::ExprKind::KvHas(store, Box::new(key_ref())),
            ty: Type::Bool,
            span,
        };
        let get = hir::Expr {
            kind: hir::ExprKind::KvGet(store, Box::new(key_ref())),
            ty: val_ty.clone(),
            span,
        };
        let some = hir::Expr {
            kind: hir::ExprKind::VariantCtor(
                opt_enum,
                "Some".into(),
                some_tag,
                vec![hir::FieldInit {
                    name: None,
                    value: get,
                }],
            ),
            ty: opt_ty.clone(),
            span,
        };
        let nothing = hir::Expr {
            kind: hir::ExprKind::VariantCtor(opt_enum, "Nothing".into(), nothing_tag, vec![]),
            ty: opt_ty.clone(),
            span,
        };
        let ternary = hir::Expr {
            kind: hir::ExprKind::Ternary(Box::new(has), Box::new(some), Box::new(nothing)),
            ty: opt_ty.clone(),
            span,
        };
        Ok(hir::Expr {
            kind: hir::ExprKind::Block(vec![key_bind, hir::Stmt::Expr(ternary)]),
            ty: opt_ty,
            span,
        })
    }

    pub(in crate::typer) fn dispatch_store_methods(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: crate::ast::Span,
    ) -> Result<Option<hir::Expr>, String> {
        if let Some(e) = self.try_store_group(obj, method, args, span)? {
            return Ok(Some(e));
        }

        if let ast::Expr::Ident(name, _) = obj
            && self.store_schemas.contains_key(&name.as_str())
        {
            let is_kv = self
                .store_decorators
                .get(&name.as_str())
                .map(|decs| decs.contains(&crate::ast::StoreDecorator::Kv))
                .unwrap_or(false);
            if is_kv {
                let (key_ty, val_ty) = self.kv_key_val_types(&name.as_str());
                if !matches!(key_ty, Type::String) {
                    return Err(format!(
                        "@kv store '{name}' key field must be `String`, found `{key_ty}`"
                    ));
                }
                let val_is_num = matches!(val_ty, Type::I64 | Type::F64);
                if !val_is_num {
                    return Err(format!(
                        "@kv store '{name}' value type `{val_ty}` is not supported; declare `val as i64` or `val as f64`"
                    ));
                }
                let val_is_int = matches!(val_ty, Type::I64);
                match method {
                    "set" => {
                        if args.len() != 2 {
                            return Err("kv.set() requires 2 arguments (key, value)".into());
                        }
                        let key_expr = self.lower_expr_expected(&args[0], Some(&Type::String))?;
                        let val_expr = self.lower_expr_expected(&args[1], Some(&val_ty))?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::KvSet(
                                *name,
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
                        let key_expr = self.lower_expr_expected(&args[0], Some(&Type::String))?;
                        return Ok(Some(
                            self.build_kv_get_option(*name, key_expr, &val_ty, span)?,
                        ));
                    }
                    "has" => {
                        if args.len() != 1 {
                            return Err("kv.has() requires 1 argument (key)".into());
                        }
                        let key_expr = self.lower_expr_expected(&args[0], Some(&Type::String))?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::KvHas(*name, Box::new(key_expr)),
                            ty: Type::Bool,
                            span,
                        }));
                    }
                    "count" => {
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::KvCount(*name),
                            ty: Type::I64,
                            span,
                        }));
                    }
                    "del" => {
                        if args.len() != 1 {
                            return Err("kv.del() requires 1 argument (key)".into());
                        }
                        let key_expr = self.lower_expr_expected(&args[0], Some(&Type::String))?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::KvDel(*name, Box::new(key_expr)),
                            ty: Type::Void,
                            span,
                        }));
                    }
                    "incr" | "decr" if !val_is_int => {
                        return Err(format!(
                            "kv.{method}() requires an integer value type; store '{name}' has `val as {val_ty}`"
                        ));
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
                            return Err("kv.incr() requires 1-2 arguments (key [, delta])".into());
                        };
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::KvIncr(
                                *name,
                                Box::new(key_expr),
                                Box::new(delta_expr),
                            ),
                            ty: Type::Void,
                            span,
                        }));
                    }
                    "decr" => {
                        let key_expr = if !args.is_empty() {
                            self.lower_expr_expected(&args[0], Some(&Type::String))?
                        } else {
                            return Err("kv.decr() requires 1-2 arguments (key [, delta])".into());
                        };
                        let delta_expr = if args.len() == 2 {
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
                                *name,
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

            let is_graph = self
                .store_decorators
                .get(&name.as_str())
                .map(|decs| decs.contains(&crate::ast::StoreDecorator::Graph))
                .unwrap_or(false);
            if is_graph {
                match method {
                    "from" => {
                        if args.len() != 1 {
                            return Err("graph.from() requires 1 argument (node)".into());
                        }

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
                        let neighbor_ty = user_fields
                            .get(1)
                            .map(|(_, t)| t.clone())
                            .unwrap_or(Type::I64);
                        let node_expr = self.lower_expr_expected(&args[0], Some(&first_ty))?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::GraphFrom(*name, Box::new(node_expr)),
                            ty: Type::Vec(Box::new(neighbor_ty)),
                            span,
                        }));
                    }
                    "to" => {
                        if args.len() != 1 {
                            return Err("graph.to() requires 1 argument (node)".into());
                        }

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
                        let neighbor_ty = user_fields
                            .first()
                            .map(|(_, t)| t.clone())
                            .unwrap_or(Type::I64);
                        let node_expr = self.lower_expr_expected(&args[0], Some(&second_ty))?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::GraphTo(*name, Box::new(node_expr)),
                            ty: Type::Vec(Box::new(neighbor_ty)),
                            span,
                        }));
                    }
                    _ => {}
                }
            }

            let is_ts = self
                .store_decorators
                .get(&name.as_str())
                .map(|decs| {
                    decs.iter()
                        .any(|d| matches!(d, crate::ast::StoreDecorator::TimeSeries(_)))
                })
                .unwrap_or(false);
            if is_ts && method == "latest" {
                if !args.is_empty() {
                    return Err("timeseries.latest() takes no arguments".into());
                }
                return Ok(Some(hir::Expr {
                    kind: hir::ExprKind::TsLatest(*name),
                    ty: Type::I64,
                    span,
                }));
            }

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
                                "vector.nearest() requires 2 arguments (query_array, k)".into()
                            );
                        }
                        let query_expr = self.lower_expr(&args[0])?;
                        let k_expr = self.lower_expr(&args[1])?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::VecNearest(
                                *name,
                                Box::new(query_expr),
                                Box::new(k_expr),
                            ),
                            ty: Type::Vec(Box::new(Type::Tuple(vec![Type::I64, Type::F64]))),
                            span,
                        }));
                    }
                    "insert" => {
                        if args.len() != 1 {
                            return Err("vector.insert() requires 1 argument (vector array)".into());
                        }
                        let vec_expr = self.lower_expr(&args[0])?;
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::VecInsert(*name, Box::new(vec_expr)),
                            ty: Type::I64,
                            span,
                        }));
                    }
                    "count" => {
                        if !args.is_empty() {
                            return Err("vector.count() takes no arguments".into());
                        }
                        return Ok(Some(hir::Expr {
                            kind: hir::ExprKind::VecCount(*name),
                            ty: Type::I64,
                            span,
                        }));
                    }
                    _ => {}
                }
            }

            if method == "maybe" {
                if args.len() != 2 {
                    return Err("maybe() requires 2 arguments (field_name, value)".into());
                }
                let field = match &args[0] {
                    ast::Expr::Ident(f, _) => *f,
                    _ => return Err("maybe() first argument must be a field name".into()),
                };
                let val_expr = self.lower_expr(&args[1])?;
                return Ok(Some(hir::Expr {
                    kind: hir::ExprKind::BloomTest(*name, field, Box::new(val_expr)),
                    ty: Type::Bool,
                    span,
                }));
            }

            if method == "search" {
                if args.len() != 2 {
                    return Err("search() requires 2 arguments (field_name, query)".into());
                }
                let field = match &args[0] {
                    ast::Expr::Ident(f, _) => *f,
                    _ => return Err("search() first argument must be a field name".into()),
                };
                let query_expr = self.lower_expr(&args[1])?;
                return Ok(Some(hir::Expr {
                    kind: hir::ExprKind::FtsSearch(*name, field, Box::new(query_expr)),
                    ty: Type::Vec(Box::new(Type::I64)),
                    span,
                }));
            }
            if method == "search_count" {
                if args.len() != 1 {
                    return Err("search_count() requires 1 argument (field_name)".into());
                }
                let field = match &args[0] {
                    ast::Expr::Ident(f, _) => *f,
                    _ => return Err("search_count() argument must be a field name".into()),
                };
                return Ok(Some(hir::Expr {
                    kind: hir::ExprKind::FtsCount(*name, field),
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
                        ast::Expr::Ident(f, _) => *f,
                        _ => return Err(format!("{method}() argument must be a field name")),
                    };
                    let kind = match method {
                        "sum" => hir::ExprKind::StoreSum(*name, field),
                        "avg" => hir::ExprKind::StoreAvg(*name, field),
                        "min" => hir::ExprKind::StoreMin(*name, field),
                        "max" => hir::ExprKind::StoreMax(*name, field),
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
                        ast::Expr::Ident(f, _) => *f,
                        _ => return Err("distinct() argument must be a field name".into()),
                    };
                    let field_ty = self
                        .store_schemas
                        .get(&name.as_str())
                        .and_then(|schema| {
                            schema
                                .iter()
                                .find(|(n, _)| n == &field)
                                .map(|(_, t)| t.clone())
                        })
                        .ok_or_else(|| {
                            format!("distinct(): unknown field '{field}' in store '{name}'")
                        })?;
                    return Ok(Some(hir::Expr {
                        kind: hir::ExprKind::StoreDistinct(*name, field),
                        ty: Type::Vec(Box::new(field_ty)),
                        span,
                    }));
                }
                "count" => {
                    return Ok(Some(hir::Expr {
                        kind: hir::ExprKind::StoreCount(*name),
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
                        kind: hir::ExprKind::StoreVersionCount(*name, Box::new(sid_expr)),
                        ty: Type::I64,
                        span,
                    }));
                }
                "history" => {
                    if args.len() != 1 {
                        return Err("history() requires exactly 1 argument (sid)".into());
                    }
                    let sid_expr = self.lower_expr_expected(&args[0], Some(&Type::I64))?;
                    let struct_name = crate::intern::Symbol::intern(&format!("__store_{name}"));
                    return Ok(Some(hir::Expr {
                        kind: hir::ExprKind::StoreHistory(*name, Box::new(sid_expr)),
                        ty: Type::Vec(Box::new(Type::Struct(struct_name, vec![]))),
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
                            *name,
                            Box::new(sid_expr),
                            Box::new(ver_expr),
                        ),
                        ty: Type::I64,
                        span,
                    }));
                }
                _ => {}
            }
        }

        Ok(None)
    }

    fn try_store_group(
        &mut self,
        obj: &ast::Expr,
        method: &str,
        args: &[ast::Expr],
        span: crate::ast::Span,
    ) -> Result<Option<hir::Expr>, String> {
        let agg = match method {
            "count" => hir::GroupAgg::Count,
            "sum" => hir::GroupAgg::Sum,
            "avg" => hir::GroupAgg::Avg,
            "min" => hir::GroupAgg::Min,
            "max" => hir::GroupAgg::Max,
            _ => return Ok(None),
        };

        let ast::Expr::Method(inner, inner_method, group_args, _) = obj else {
            return Ok(None);
        };
        if &*inner_method.as_str() != "group" {
            return Ok(None);
        }
        let ast::Expr::Ident(store, _) = &**inner else {
            return Ok(None);
        };
        if !self.store_schemas.contains_key(&store.as_str()) {
            return Ok(None);
        }

        if group_args.len() != 1 {
            return Err("group() requires exactly 1 field argument".into());
        }
        let key_field = match &group_args[0] {
            ast::Expr::Ident(f, _) => *f,
            _ => return Err("group() argument must be a field name".into()),
        };

        let schema = self
            .store_schemas
            .get(&store.as_str())
            .cloned()
            .ok_or_else(|| format!("unknown store '{store}'"))?;
        let field_ty = |fld: &crate::intern::Symbol| {
            schema
                .iter()
                .find(|(n, _)| n == fld)
                .map(|(_, t)| t.clone())
        };
        let key_ty = field_ty(&key_field)
            .ok_or_else(|| format!("group(): unknown field '{key_field}' in store '{store}'"))?;

        let (val_field, val_ty) = if agg == hir::GroupAgg::Count {
            if !args.is_empty() {
                return Err("count() after group() takes no arguments".into());
            }
            (None, Type::I64)
        } else {
            if args.len() != 1 {
                return Err(format!(
                    "{method}() after group() requires 1 field argument"
                ));
            }
            let vf = match &args[0] {
                ast::Expr::Ident(f, _) => *f,
                _ => return Err(format!("{method}() argument must be a field name")),
            };
            let vty = field_ty(&vf)
                .ok_or_else(|| format!("{method}(): unknown field '{vf}' in store '{store}'"))?;
            let ret = if agg == hir::GroupAgg::Avg {
                Type::F64
            } else {
                match vty {
                    Type::F64 | Type::F32 => Type::F64,
                    _ => Type::I64,
                }
            };
            (Some(vf), ret)
        };

        Ok(Some(hir::Expr {
            kind: hir::ExprKind::StoreGroup(*store, key_field, agg, val_field),
            ty: Type::Vec(Box::new(Type::Tuple(vec![key_ty, val_ty]))),
            span,
        }))
    }
}
