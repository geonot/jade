//! Extracted lowering steps.

#![allow(unused_imports, unused_variables)]

use std::collections::{HashMap, HashSet};

use super::super::unify;
use super::super::{DeferredField, DeferredMethod, Typer, VarInfo};
use crate::ast::{self, Span};
use crate::hir::{self, CoercionKind, DefId, ExprKind, Ownership};
use crate::intern::Symbol;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn iter_element_type(&self, type_name: &str) -> Type {
        if let Some(args) = self
            .trait_impl_type_args
            .get(&(type_name.into(), "Iter".into()))
        {
            if let Some(t) = args.first() {
                return t.clone();
            }
        }
        let fn_name = format!("{type_name}_next");
        if let Some((_, _, ret)) = self.fns.get(&fn_name) {
            if let Type::Enum(ename) = ret {
                if let Some(stripped) = ename.strip_prefix("Option_") {
                    return match &*stripped.as_str() {
                        "i64" => Type::I64,
                        "f64" => Type::F64,
                        "bool" => Type::Bool,
                        "String" => Type::String,
                        other => Type::Struct(other.into(), vec![]),
                    };
                }
            }
        }
        Type::I64
    }

    pub(in crate::typer) fn desugar_for_iter(
        &mut self,
        f: &ast::For,
        iter_expr: hir::Expr,
        type_name: String,
        elem_ty: Type,
        ret_ty: &Type,
    ) -> Result<hir::Stmt, String> {
        let span = f.span;

        let mut option_type_map = HashMap::new();
        option_type_map.insert("T".into(), elem_ty.clone());
        let option_enum_name = self.monomorphize_enum("Option", &option_type_map)?;

        let some_tag = self.variant_tags.get("Some").map(|(_, t)| *t).unwrap_or(0);
        let nothing_tag = self
            .variant_tags
            .get("Nothing")
            .map(|(_, t)| *t)
            .unwrap_or(1);

        let iter_bind_id = self.fresh_id();
        let iter_var_name = format!("__iter_{}", f.bind);

        self.define_var(
            &iter_var_name,
            VarInfo {
                def_id: iter_bind_id,
                ty: iter_expr.ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );

        let bind_stmt = hir::Stmt::Bind(hir::Bind {
            def_id: iter_bind_id,
            name: Symbol::intern(&iter_var_name),
            value: iter_expr.clone(),
            ty: iter_expr.ty.clone(),
            ownership: Ownership::Owned,
            atomic: false,
            span,
        });

        let method_name = format!("{type_name}_next");
        let ret = Type::Enum(option_enum_name.into());
        if let Some(entry) = self.fns.get_mut(&method_name) {
            entry.2 = ret.clone();
        }

        let next_call = hir::Expr {
            kind: hir::ExprKind::IterNext(
                Symbol::intern(&iter_var_name),
                type_name.into(),
                "next".into(),
            ),
            ty: ret,
            span,
        };

        let bind_id = self.fresh_id();
        let some_pat = hir::Pat::Ctor(
            "Some".into(),
            some_tag,
            vec![hir::Pat::Bind(
                bind_id,
                f.bind.clone(),
                elem_ty.clone(),
                span,
            )],
            span,
        );
        let nothing_pat = hir::Pat::Ctor("Nothing".into(), nothing_tag, vec![], span);

        self.push_scope();
        self.define_var(
            &f.bind.as_str(),
            VarInfo {
                def_id: bind_id,
                ty: elem_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let body = self.lower_block_no_scope(&f.body, ret_ty)?;
        self.pop_scope();

        let some_arm = hir::Arm {
            pat: some_pat,
            guard: None,
            body,
            span,
        };
        let nothing_arm = hir::Arm {
            pat: nothing_pat,
            guard: None,
            body: vec![hir::Stmt::Break(None, span)],
            span,
        };

        let match_stmt = hir::Stmt::Match(hir::Match {
            subject: next_call,
            arms: vec![some_arm, nothing_arm],
            ty: Type::Void,
            span,
        });

        let loop_stmt = hir::Stmt::Loop(hir::Loop {
            body: vec![match_stmt],
            span,
        });

        Ok(hir::Stmt::Expr(hir::Expr {
            kind: hir::ExprKind::Block(vec![bind_stmt, loop_stmt]),
            ty: Type::Void,
            span,
        }))
    }

    /// Desugar `for k, v in map` into keys-based iteration:
    /// `__keys = map.keys(); for __i from 0 to __keys.len() { k = __keys.get(__i); v = map.get(k); ...body }`
    pub(in crate::typer) fn desugar_for_map(
        &mut self,
        f: &ast::For,
        val_bind: &str,
        map_expr: hir::Expr,
        key_ty: &Type,
        val_ty: &Type,
        ret_ty: &Type,
    ) -> Result<hir::Stmt, String> {
        let span = f.span;
        let key_ty = key_ty.clone();
        let val_ty = val_ty.clone();

        // Bind the map to a temp variable
        let map_id = self.fresh_id();
        let map_var = "__map_iter".to_string();
        let map_ty = map_expr.ty.clone();
        self.define_var(
            &map_var,
            VarInfo {
                def_id: map_id,
                ty: map_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let map_bind = hir::Stmt::Bind(hir::Bind {
            def_id: map_id,
            name: Symbol::intern(&map_var),
            value: map_expr,
            ty: map_ty.clone(),
            ownership: Ownership::Owned,
            atomic: false,
            span,
        });

        // __keys = map.keys()
        let keys_id = self.fresh_id();
        let keys_var = "__map_keys".to_string();
        let keys_ty = Type::Vec(Box::new(key_ty.clone()));
        let keys_call = hir::Expr {
            kind: hir::ExprKind::MapMethod(
                Box::new(hir::Expr {
                    kind: hir::ExprKind::Var(map_id, Symbol::intern(&map_var)),
                    ty: map_ty.clone(),
                    span,
                }),
                "keys".into(),
                vec![],
            ),
            ty: keys_ty.clone(),
            span,
        };
        self.define_var(
            &keys_var,
            VarInfo {
                def_id: keys_id,
                ty: keys_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let keys_bind = hir::Stmt::Bind(hir::Bind {
            def_id: keys_id,
            name: Symbol::intern(&keys_var),
            value: keys_call,
            ty: keys_ty.clone(),
            ownership: Ownership::Owned,
            atomic: false,
            span,
        });

        // for __i from 0 to __keys.len() { k = __keys.get(__i); v = map.get(k); ...body }
        let i_id = self.fresh_id();
        let i_var = "__map_i".to_string();
        self.push_scope();
        self.define_var(
            &i_var,
            VarInfo {
                def_id: i_id,
                ty: Type::I64,
                ownership: Ownership::Owned,
                scheme: None,
            },
        );

        // k = __keys.get(__i)
        let k_id = self.fresh_id();
        let k_get = hir::Expr {
            kind: hir::ExprKind::VecMethod(
                Box::new(hir::Expr {
                    kind: hir::ExprKind::Var(keys_id, Symbol::intern(&keys_var)),
                    ty: keys_ty.clone(),
                    span,
                }),
                "get".into(),
                vec![hir::Expr {
                    kind: hir::ExprKind::Var(i_id, Symbol::intern(&i_var)),
                    ty: Type::I64,
                    span,
                }],
            ),
            ty: key_ty.clone(),
            span,
        };
        self.define_var(
            &f.bind.as_str(),
            VarInfo {
                def_id: k_id,
                ty: key_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let k_bind = hir::Stmt::Bind(hir::Bind {
            def_id: k_id,
            name: f.bind.clone(),
            value: k_get,
            ty: key_ty.clone(),
            ownership: Ownership::Owned,
            atomic: false,
            span,
        });

        // v = map.get(k)
        let v_id = self.fresh_id();
        let v_get = hir::Expr {
            kind: hir::ExprKind::MapMethod(
                Box::new(hir::Expr {
                    kind: hir::ExprKind::Var(map_id, Symbol::intern(&map_var)),
                    ty: map_ty,
                    span,
                }),
                "get".into(),
                vec![hir::Expr {
                    kind: hir::ExprKind::Var(k_id, f.bind.clone()),
                    ty: key_ty,
                    span,
                }],
            ),
            ty: val_ty.clone(),
            span,
        };
        self.define_var(
            val_bind,
            VarInfo {
                def_id: v_id,
                ty: val_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let v_bind = hir::Stmt::Bind(hir::Bind {
            def_id: v_id,
            name: val_bind.into(),
            value: v_get,
            ty: val_ty,
            ownership: Ownership::Owned,
            atomic: false,
            span,
        });

        let user_body = self.lower_block_no_scope(&f.body, ret_ty)?;
        self.pop_scope();

        let mut for_body = vec![k_bind, v_bind];
        for_body.extend(user_body);

        // __keys.len() as the end expression
        let keys_len = hir::Expr {
            kind: hir::ExprKind::VecMethod(
                Box::new(hir::Expr {
                    kind: hir::ExprKind::Var(keys_id, Symbol::intern(&keys_var)),
                    ty: keys_ty,
                    span,
                }),
                "len".into(),
                vec![],
            ),
            ty: Type::I64,
            span,
        };

        let for_stmt = hir::Stmt::For(hir::For {
            bind_id: i_id,
            bind: Symbol::intern(&i_var),
            bind_ty: Type::I64,
            bind2_id: None,
            bind2: None,
            bind2_ty: None,
            iter: hir::Expr {
                kind: hir::ExprKind::Int(0),
                ty: Type::I64,
                span,
            },
            end: Some(keys_len),
            step: None,
            body: for_body,
            label: None,
            span,
        });

        Ok(hir::Stmt::Expr(hir::Expr {
            kind: hir::ExprKind::Block(vec![map_bind, keys_bind, for_stmt]),
            ty: Type::Void,
            span,
        }))
    }
}
