//! Name resolution: pre-declaration of functions, types, enums, actors,
//! traits, impl blocks, and prelude types.

use crate::ast::{self, BinOp, Span};
use crate::types::Type;

use super::Typer;

impl Typer {
    pub(crate) fn register_prelude_types(&mut self) {
        let s = Span::dummy();
        self.generic_enums
            .entry("Option".into())
            .or_insert_with(|| ast::EnumDef {
                name: "Option".into(),
                type_params: vec!["T".into()],
                variants: vec![
                    ast::Variant {
                        name: "Some".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("T".into()),
                        }],
                        span: s,
                    },
                    ast::Variant {
                        name: "Nothing".into(),
                        fields: vec![],
                        span: s,
                    },
                ],
                span: s,
            });
        self.generic_enums
            .entry("Result".into())
            .or_insert_with(|| ast::EnumDef {
                name: "Result".into(),
                type_params: vec!["T".into(), "E".into()],
                variants: vec![
                    ast::Variant {
                        name: "Ok".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("T".into()),
                        }],
                        span: s,
                    },
                    ast::Variant {
                        name: "Err".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("E".into()),
                        }],
                        span: s,
                    },
                ],
                span: s,
            });
    }

    pub(crate) fn declare_fn_sig(&mut self, f: &ast::Fn) {
        let ptys: Vec<Type> = f
            .params
            .iter()
            .map(|p| p.ty.clone().unwrap_or(Type::Inferred))
            .collect();
        let ret = if f.name == "main" {
            Type::I32
        } else {
            f.ret.clone().unwrap_or_else(|| self.infer_ret_ast(f))
        };
        let id = self.fresh_id();
        self.fns.insert(f.name.clone(), (id, ptys, ret));
    }

    pub(crate) fn declare_method_sig(&mut self, type_name: &str, m: &ast::Fn) {
        let method_name = format!("{type_name}_{}", m.name);
        let self_ty = Type::Struct(type_name.to_string());
        let mut ptys = vec![self_ty];
        for p in &m.params {
            ptys.push(p.ty.clone().unwrap_or(Type::Inferred));
        }
        let ret = m.ret.clone().unwrap_or_else(|| self.infer_ret_ast(m));
        let id = self.fresh_id();
        self.fns.insert(method_name, (id, ptys, ret));
    }

    pub(crate) fn declare_type_def(&mut self, td: &ast::TypeDef) {
        let fields: Vec<(String, Type)> = td
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)),
                )
            })
            .collect();
        self.structs.insert(td.name.clone(), fields);
    }

    pub(crate) fn declare_enum_def(&mut self, ed: &ast::EnumDef) {
        let mut variants = Vec::new();
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys: Vec<Type> = v.fields.iter().map(|f| f.ty.clone()).collect();
            self.variant_tags
                .insert(v.name.clone(), (ed.name.clone(), tag as u32));
            variants.push((v.name.clone(), ftys));
        }
        self.enums.insert(ed.name.clone(), variants);
    }

    pub(crate) fn declare_extern_sig(&mut self, ef: &ast::ExternFn) {
        let ptys: Vec<Type> = ef.params.iter().map(|(_, t)| t.clone()).collect();
        let id = self.fresh_id();
        self.fns.insert(ef.name.clone(), (id, ptys, ef.ret.clone()));
    }

    pub(crate) fn declare_err_def_sig(&mut self, ed: &ast::ErrDef) {
        let mut variants = Vec::new();
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys = v.fields.clone();
            self.variant_tags
                .insert(v.name.clone(), (ed.name.clone(), tag as u32));
            variants.push((v.name.clone(), ftys));
        }
        self.enums.insert(ed.name.clone(), variants);
    }

    pub(crate) fn declare_actor_def(&mut self, ad: &ast::ActorDef) {
        let id = self.fresh_id();
        let fields: Vec<(String, Type)> = ad
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)),
                )
            })
            .collect();
        let handlers: Vec<(String, Vec<Type>, u32)> = ad
            .handlers
            .iter()
            .enumerate()
            .map(|(tag, h)| {
                let ptys: Vec<Type> = h
                    .params
                    .iter()
                    .map(|p| p.ty.clone().unwrap_or(Type::Inferred))
                    .collect();
                (h.name.clone(), ptys, tag as u32)
            })
            .collect();
        self.actors
            .insert(ad.name.clone(), (id, fields, handlers));
    }

    pub(crate) fn declare_trait_def(&mut self, td: &ast::TraitDef) {
        let sigs: Vec<super::TraitMethodSig> = td
            .methods
            .iter()
            .map(|m| super::TraitMethodSig {
                name: m.name.clone(),
                _params: m
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone()))
                    .collect(),
                _ret: m.ret.clone(),
                has_default: m.default_body.is_some(),
            })
            .collect();
        self.traits.insert(td.name.clone(), sigs);
    }

    pub(crate) fn declare_impl_block(&mut self, ib: &ast::ImplBlock) -> Result<(), String> {
        if !self.structs.contains_key(&ib.type_name) {
            return Err(format!(
                "line {}: impl references unknown type '{}'",
                ib.span.line, ib.type_name
            ));
        }

        if let Some(ref trait_name) = ib.trait_name {
            if !self.traits.contains_key(trait_name) {
                return Err(format!(
                    "line {}: impl references unknown trait '{}'",
                    ib.span.line, trait_name
                ));
            }

            // Verify all required trait methods are provided
            let trait_sigs = self.traits.get(trait_name).cloned().unwrap();
            let impl_method_names: Vec<&str> = ib.methods.iter().map(|m| m.name.as_str()).collect();
            for sig in &trait_sigs {
                if !sig.has_default && !impl_method_names.contains(&sig.name.as_str()) {
                    return Err(format!(
                        "line {}: impl {} for {} is missing required method '{}'",
                        ib.span.line, trait_name, ib.type_name, sig.name
                    ));
                }
            }

            self.trait_impls
                .entry(ib.type_name.clone())
                .or_default()
                .push(trait_name.clone());
        }

        // Register impl methods as type methods (same as inline methods)
        for m in &ib.methods {
            self.methods
                .entry(ib.type_name.clone())
                .or_default()
                .push(m.clone());
            self.declare_method_sig(&ib.type_name, m);
        }

        Ok(())
    }

    // ── Bidirectional parameter type inference ──────────────────────────
    //
    // After all signatures are declared (some with Type::Inferred slots),
    // we iteratively refine them by looking at:
    //   1. Body-driven: how params are used inside the function body
    //   2. Call-site-driven: what types callers pass in
    //
    // Iterate until no more changes (fixed-point), then default any
    // remaining Inferred slots to i64.

    pub(crate) fn infer_param_types(&mut self, prog: &ast::Program) {
        // Collect all functions with their AST bodies for inference
        let mut fn_asts: Vec<(String, Vec<String>, &[ast::Stmt])> = Vec::new();
        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) if !Self::is_generic_fn(f) => {
                    let pnames: Vec<String> = f.params.iter().map(|p| p.name.clone()).collect();
                    fn_asts.push((f.name.clone(), pnames, &f.body));
                }
                ast::Decl::Type(td) if td.type_params.is_empty() => {
                    for m in &td.methods {
                        let method_name = format!("{}_{}", td.name, m.name);
                        // skip "self" — it's always typed
                        let pnames: Vec<String> = m.params.iter().map(|p| p.name.clone()).collect();
                        fn_asts.push((method_name, pnames, &m.body));
                    }
                }
                ast::Decl::Impl(ib) => {
                    for m in &ib.methods {
                        let method_name = format!("{}_{}", ib.type_name, m.name);
                        let pnames: Vec<String> = m.params.iter().map(|p| p.name.clone()).collect();
                        fn_asts.push((method_name, pnames, &m.body));
                    }
                }
                _ => {}
            }
        }

        // Fixed-point iteration: keep refining until stable
        for _ in 0..8 {
            let mut changed = false;

            // Body-driven: infer from how params are used in the function body
            for (fn_name, pnames, body) in &fn_asts {
                let sig = match self.fns.get(fn_name.as_str()) {
                    Some(s) => s.clone(),
                    None => continue,
                };
                // offset: methods have self prepended
                let offset = sig.1.len().saturating_sub(pnames.len());

                for (pi, pname) in pnames.iter().enumerate() {
                    let slot = offset + pi;
                    if slot >= sig.1.len() || sig.1[slot] != Type::Inferred {
                        continue;
                    }
                    if let Some(ty) = self.infer_param_from_body(pname, body) {
                        if let Some(entry) = self.fns.get_mut(fn_name.as_str()) {
                            entry.1[slot] = ty;
                            changed = true;
                        }
                    }
                }
            }

            // Call-site-driven: infer from what types callers pass in
            for (_fn_name, _pnames, body) in &fn_asts {
                changed |= self.infer_from_call_sites(body);
            }

            if !changed {
                break;
            }
        }

        // Re-infer return types now that params are resolved.
        // Functions whose return type was inferred before param types
        // were known may have gotten wrong results.
        for d in &prog.decls {
            if let ast::Decl::Fn(f) = d {
                if Self::is_generic_fn(f) { continue; }
                if f.ret.is_some() || f.name == "main" { continue; }
                // Re-run return type inference with updated param knowledge
                let ret = self.infer_ret_ast_with_params(f, &f.name.clone());
                if let Some(entry) = self.fns.get_mut(&f.name) {
                    entry.2 = ret;
                }
            }
            if let ast::Decl::Type(td) = d {
                if !td.type_params.is_empty() { continue; }
                for m in &td.methods {
                    if m.ret.is_some() { continue; }
                    let method_name = format!("{}_{}", td.name, m.name);
                    let ret = self.infer_ret_ast_with_params(m, &method_name);
                    if let Some(entry) = self.fns.get_mut(&method_name) {
                        entry.2 = ret;
                    }
                }
            }
            if let ast::Decl::Impl(ib) = d {
                for m in &ib.methods {
                    if m.ret.is_some() { continue; }
                    let method_name = format!("{}_{}", ib.type_name, m.name);
                    let ret = self.infer_ret_ast_with_params(m, &method_name);
                    if let Some(entry) = self.fns.get_mut(&method_name) {
                        entry.2 = ret;
                    }
                }
            }
        }

        // Default any remaining Inferred slots to i64
        let keys: Vec<String> = self.fns.keys().cloned().collect();
        for k in keys {
            let entry = self.fns.get_mut(&k).unwrap();
            for ty in &mut entry.1 {
                if *ty == Type::Inferred {
                    *ty = Type::I64;
                }
            }
        }
    }

    /// Walk statements looking for constraints on `param_name`.
    fn infer_param_from_body(&self, param_name: &str, body: &[ast::Stmt]) -> Option<Type> {
        let mut result = None;
        for stmt in body {
            if let Some(ty) = self.constraint_from_stmt(param_name, stmt) {
                result = Some(ty);
                break;
            }
        }
        result
    }

    fn constraint_from_stmt(&self, name: &str, stmt: &ast::Stmt) -> Option<Type> {
        match stmt {
            ast::Stmt::Expr(e) => self.constraint_from_expr(name, e),
            ast::Stmt::Bind(b) => self.constraint_from_expr(name, &b.value),
            ast::Stmt::Assign(_, rhs, _) => self.constraint_from_expr(name, rhs),
            ast::Stmt::Ret(Some(e), _) => self.constraint_from_expr(name, e),
            ast::Stmt::If(i) => {
                if let Some(t) = self.constraint_from_expr(name, &i.cond) { return Some(t); }
                if let Some(t) = self.constraint_from_body(name, &i.then) { return Some(t); }
                for (c, b) in &i.elifs {
                    if let Some(t) = self.constraint_from_expr(name, c) { return Some(t); }
                    if let Some(t) = self.constraint_from_body(name, b) { return Some(t); }
                }
                if let Some(b) = &i.els {
                    return self.constraint_from_body(name, b);
                }
                None
            }
            ast::Stmt::While(w) => {
                if let Some(t) = self.constraint_from_expr(name, &w.cond) { return Some(t); }
                self.constraint_from_body(name, &w.body)
            }
            ast::Stmt::For(f) => {
                if let Some(t) = self.constraint_from_expr(name, &f.iter) { return Some(t); }
                self.constraint_from_body(name, &f.body)
            }
            ast::Stmt::Loop(l) => self.constraint_from_body(name, &l.body),
            ast::Stmt::Match(m) => {
                if let Some(t) = self.constraint_from_expr(name, &m.subject) { return Some(t); }
                for arm in &m.arms {
                    if let Some(t) = self.constraint_from_body(name, &arm.body) { return Some(t); }
                }
                None
            }
            ast::Stmt::ErrReturn(e, _) => self.constraint_from_expr(name, e),
            _ => None,
        }
    }

    fn constraint_from_body(&self, name: &str, body: &[ast::Stmt]) -> Option<Type> {
        for stmt in body {
            if let Some(ty) = self.constraint_from_stmt(name, stmt) {
                return Some(ty);
            }
        }
        None
    }

    /// Extract a type constraint on `name` from an expression.
    fn constraint_from_expr(&self, name: &str, expr: &ast::Expr) -> Option<Type> {
        match expr {
            // Direct call: f(name) — adopt f's param type at that position
            ast::Expr::Call(callee, args, _) => {
                if let ast::Expr::Ident(fn_name, _) = callee.as_ref() {
                    // Check registered functions first, then builtins
                    let ptys: Option<Vec<Type>> = self.fns.get(fn_name.as_str())
                        .map(|(_, ptys, _)| ptys.clone())
                        .or_else(|| Self::builtin_param_tys(fn_name));
                    if let Some(ptys) = ptys {
                        for (i, arg) in args.iter().enumerate() {
                            if Self::expr_is_ident(arg, name) {
                                if let Some(pty) = ptys.get(i) {
                                    if *pty != Type::Inferred {
                                        return Some(pty.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                // Recurse into args
                for arg in args {
                    if let Some(t) = self.constraint_from_expr(name, arg) { return Some(t); }
                }
                self.constraint_from_expr(name, callee)
            }

            // Method call: name.method(...) — infer from method receiver patterns
            ast::Expr::Method(obj, method, args, _) => {
                if Self::expr_is_ident(obj, name) {
                    match method.as_str() {
                        "length" | "char_at" | "slice" | "contains" | "starts_with"
                        | "ends_with" | "split" | "trim" | "to_upper" | "to_lower"
                        | "replace" | "index_of" | "find" => return Some(Type::String),
                        "get" | "set" | "push" | "pop" | "remove" | "insert" | "sort"
                        | "reverse" | "clear" | "extend" | "map" | "filter" | "reduce"
                        | "any" | "all" | "flat_map" | "zip" => {
                            // Could be Vec — but we need element type info.
                            // length is ambiguous (String or Vec), handled by String first
                            // since .get/.push are Vec-specific, try to figure out element type
                        }
                        _ => {
                            // Check if it's a known struct method
                            // name.method -> name is some Struct type
                        }
                    }
                }
                // Check if name is passed as an argument to the method
                if let ast::Expr::Ident(obj_name, _) = obj.as_ref() {
                    if let Some(v) = self.find_var(obj_name) {
                        if let Type::Struct(type_name) = &v.ty {
                            let method_name = format!("{type_name}_{method}");
                            if let Some((_, ptys, _)) = self.fns.get(&method_name) {
                                // ptys[0] is self, args start at ptys[1]
                                for (i, arg) in args.iter().enumerate() {
                                    if Self::expr_is_ident(arg, name) {
                                        if let Some(pty) = ptys.get(i + 1) {
                                            if *pty != Type::Inferred {
                                                return Some(pty.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for arg in args {
                    if let Some(t) = self.constraint_from_expr(name, arg) { return Some(t); }
                }
                self.constraint_from_expr(name, obj)
            }

            // BinOp: name + 1.0 => f64, name + "s" => String
            ast::Expr::BinOp(l, op, r, _) => {
                let is_arith = matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod);
                if is_arith {
                    if Self::expr_is_ident(l, name) {
                        let rty = self.expr_ty_ast(r);
                        if rty != Type::I64 && rty != Type::Inferred {
                            return Some(rty);
                        }
                    }
                    if Self::expr_is_ident(r, name) {
                        let lty = self.expr_ty_ast(l);
                        if lty != Type::I64 && lty != Type::Inferred {
                            return Some(lty);
                        }
                    }
                }
                if let Some(t) = self.constraint_from_expr(name, l) { return Some(t); }
                self.constraint_from_expr(name, r)
            }

            // Ternary: cond ? then : else
            ast::Expr::Ternary(c, t, f, _) => {
                if let Some(ty) = self.constraint_from_expr(name, c) { return Some(ty); }
                if let Some(ty) = self.constraint_from_expr(name, t) { return Some(ty); }
                self.constraint_from_expr(name, f)
            }

            // Recurse into nested expressions
            ast::Expr::UnaryOp(_, inner, _) => {
                self.constraint_from_expr(name, inner)
            }
            ast::Expr::Ref(inner, _) | ast::Expr::Deref(inner, _) => {
                self.constraint_from_expr(name, inner)
            }
            ast::Expr::Index(arr, idx, _) => {
                if let Some(t) = self.constraint_from_expr(name, arr) { return Some(t); }
                self.constraint_from_expr(name, idx)
            }
            ast::Expr::IfExpr(i) => {
                if let Some(t) = self.constraint_from_expr(name, &i.cond) { return Some(t); }
                if let Some(t) = self.constraint_from_body(name, &i.then) { return Some(t); }
                for (c, b) in &i.elifs {
                    if let Some(t) = self.constraint_from_expr(name, c) { return Some(t); }
                    if let Some(t) = self.constraint_from_body(name, b) { return Some(t); }
                }
                if let Some(b) = &i.els {
                    return self.constraint_from_body(name, b);
                }
                None
            }
            ast::Expr::Pipe(input, _, _, _) => self.constraint_from_expr(name, input),
            ast::Expr::Block(stmts, _) => self.constraint_from_body(name, stmts),
            ast::Expr::As(inner, _, _) => self.constraint_from_expr(name, inner),
            _ => None,
        }
    }

    /// Walk call sites: for `f(expr)`, if we know expr's type and f's param
    /// is Inferred, refine it. Returns true if anything changed.
    fn infer_from_call_sites(&mut self, body: &[ast::Stmt]) -> bool {
        let mut changed = false;
        for stmt in body {
            changed |= self.call_site_stmt(stmt);
        }
        changed
    }

    fn call_site_stmt(&mut self, stmt: &ast::Stmt) -> bool {
        match stmt {
            ast::Stmt::Expr(e) => self.call_site_expr(e),
            ast::Stmt::Bind(b) => self.call_site_expr(&b.value),
            ast::Stmt::Assign(_, rhs, _) => self.call_site_expr(rhs),
            ast::Stmt::Ret(Some(e), _) => self.call_site_expr(e),
            ast::Stmt::If(i) => {
                let mut c = self.call_site_expr(&i.cond);
                for s in &i.then { c |= self.call_site_stmt(s); }
                for (cond, b) in &i.elifs {
                    c |= self.call_site_expr(cond);
                    for s in b { c |= self.call_site_stmt(s); }
                }
                if let Some(b) = &i.els { for s in b { c |= self.call_site_stmt(s); } }
                c
            }
            ast::Stmt::While(w) => {
                let mut c = self.call_site_expr(&w.cond);
                for s in &w.body { c |= self.call_site_stmt(s); }
                c
            }
            ast::Stmt::For(f) => {
                let mut c = self.call_site_expr(&f.iter);
                for s in &f.body { c |= self.call_site_stmt(s); }
                c
            }
            ast::Stmt::Loop(l) => {
                let mut c = false;
                for s in &l.body { c |= self.call_site_stmt(s); }
                c
            }
            ast::Stmt::Match(m) => {
                let mut c = self.call_site_expr(&m.subject);
                for arm in &m.arms {
                    for s in &arm.body { c |= self.call_site_stmt(s); }
                }
                c
            }
            _ => false,
        }
    }

    fn call_site_expr(&mut self, expr: &ast::Expr) -> bool {
        match expr {
            ast::Expr::Call(callee, args, _) => {
                let mut c = false;
                if let ast::Expr::Ident(fn_name, _) = callee.as_ref() {
                    let fn_name = fn_name.clone();
                    if let Some((_, ptys, _)) = self.fns.get(&fn_name) {
                        let ptys = ptys.clone();
                        for (i, arg) in args.iter().enumerate() {
                            if i < ptys.len() && ptys[i] == Type::Inferred {
                                let arg_ty = self.expr_ty_ast(arg);
                                if arg_ty != Type::I64 || !matches!(arg, ast::Expr::Ident(..)) {
                                    // Only refine if we have real type info, not just default i64
                                    if arg_ty != Type::Inferred {
                                        if let Some(entry) = self.fns.get_mut(&fn_name) {
                                            entry.1[i] = arg_ty;
                                            c = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                for arg in args { c |= self.call_site_expr(arg); }
                c
            }
            ast::Expr::Method(obj, _, args, _) => {
                let mut c = self.call_site_expr(obj);
                for arg in args { c |= self.call_site_expr(arg); }
                c
            }
            ast::Expr::BinOp(l, _, r, _) => {
                self.call_site_expr(l) | self.call_site_expr(r)
            }
            ast::Expr::Ternary(a, b, c_expr, _) => {
                self.call_site_expr(a) | self.call_site_expr(b) | self.call_site_expr(c_expr)
            }
            ast::Expr::UnaryOp(_, e, _) | ast::Expr::Ref(e, _) | ast::Expr::Deref(e, _)
            | ast::Expr::As(e, _, _) => self.call_site_expr(e),
            ast::Expr::Index(a, b, _) => self.call_site_expr(a) | self.call_site_expr(b),
            ast::Expr::IfExpr(i) => {
                let mut c = self.call_site_expr(&i.cond);
                for s in &i.then { c |= self.call_site_stmt(s); }
                for (cond, b) in &i.elifs {
                    c |= self.call_site_expr(cond);
                    for s in b { c |= self.call_site_stmt(s); }
                }
                if let Some(b) = &i.els { for s in b { c |= self.call_site_stmt(s); } }
                c
            }
            ast::Expr::Block(stmts, _) => {
                let mut c = false;
                for s in stmts { c |= self.call_site_stmt(s); }
                c
            }
            ast::Expr::Pipe(a, b, extra, _) => {
                let mut c = self.call_site_expr(a) | self.call_site_expr(b);
                for e in extra { c |= self.call_site_expr(e); }
                c
            }
            _ => false,
        }
    }

    fn expr_is_ident(expr: &ast::Expr, name: &str) -> bool {
        matches!(expr, ast::Expr::Ident(n, _) if n == name)
    }
}
