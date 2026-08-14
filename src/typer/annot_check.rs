use std::collections::HashSet;

use crate::ast;
use crate::intern::Symbol;
use crate::types::Type;

use super::Typer;

impl Typer {
    pub(in crate::typer) fn check_declared_annotations(&mut self, prog: &ast::Program) {
        let mut nominal: HashSet<Symbol> = HashSet::new();
        for d in &prog.decls {
            if let ast::Decl::TypeAlias(n, _, _) | ast::Decl::Newtype(n, _, _) = d {
                nominal.insert(*n);
            }
        }
        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) => {
                    self.check_fn_annotations(f, &HashSet::new(), &nominal);
                }
                ast::Decl::Type(td) => {
                    let tparams: HashSet<Symbol> = td.type_params.iter().cloned().collect();
                    for fld in &td.fields {
                        if let Some(ty) = &fld.ty {
                            let ctx = format!("field `{}` of `{}`", fld.name, td.name);
                            self.check_annotation_ty(ty, &tparams, fld.span, &ctx, &nominal);
                        }
                    }
                    for m in &td.methods {
                        self.check_fn_annotations(m, &tparams, &nominal);
                    }
                }
                ast::Decl::Enum(ed) => {
                    let tparams: HashSet<Symbol> = ed.type_params.iter().cloned().collect();
                    for v in &ed.variants {
                        for vf in &v.fields {
                            let ctx = format!("variant `{}` of `{}`", v.name, ed.name);
                            self.check_annotation_ty(&vf.ty, &tparams, v.span, &ctx, &nominal);
                        }
                    }
                }
                ast::Decl::Actor(ad) => {
                    let empty = HashSet::new();
                    for fld in &ad.fields {
                        if let Some(ty) = &fld.ty {
                            let ctx = format!("field `{}` of actor `{}`", fld.name, ad.name);
                            self.check_annotation_ty(ty, &empty, fld.span, &ctx, &nominal);
                        }
                    }
                    for h in &ad.handlers {
                        for p in &h.params {
                            if let Some(ty) = &p.ty {
                                let ctx = format!("parameter `{}` of handler `{}`", p.name, h.name);
                                self.check_annotation_ty(ty, &empty, p.span, &ctx, &nominal);
                            }
                        }
                    }
                }
                ast::Decl::Store(sd) => {
                    let empty = HashSet::new();
                    for fld in &sd.fields {
                        if fld.is_relation {
                            continue;
                        }
                        if let Some(ty) = &fld.ty {
                            let ctx = format!("field `{}` of store `{}`", fld.name, sd.name);
                            self.check_annotation_ty(ty, &empty, fld.span, &ctx, &nominal);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn check_fn_annotations(
        &mut self,
        f: &ast::Fn,
        outer: &HashSet<Symbol>,
        nominal: &HashSet<Symbol>,
    ) {
        let mut tparams: HashSet<Symbol> = outer.clone();
        tparams.extend(f.type_params.iter().cloned());
        for (n, _) in &f.type_bounds {
            tparams.insert(*n);
        }
        for p in &f.params {
            if let Some(ty) = &p.ty {
                let ctx = format!("parameter `{}` of `{}`", p.name, f.name);
                self.check_annotation_ty(ty, &tparams, p.span, &ctx, nominal);
            }
        }
        if let Some(ret) = &f.ret {
            let ctx = format!("return type of `{}`", f.name);
            self.check_annotation_ty(ret, &tparams, f.span, &ctx, nominal);
        }
    }

    pub(in crate::typer) fn check_annotation_ty(
        &mut self,
        ty: &Type,
        tparams: &HashSet<Symbol>,
        span: ast::Span,
        ctx: &str,
        nominal: &HashSet<Symbol>,
    ) {
        match ty {
            Type::Struct(n, args) => {
                if n.as_str() == "Self" || tparams.contains(n) {
                    for a in args {
                        self.check_annotation_ty(a, tparams, span, ctx, nominal);
                    }
                    return;
                }
                let known = self.structs.contains_key(n)
                    || self.generic_types.contains_key(n)
                    || self.generic_enums.contains_key(n)
                    || self.enums.contains_key(n)
                    || self.actors.contains_key(n)
                    || self.traits.contains_key(n)
                    || self.store_schemas.contains_key(n)
                    || nominal.contains(n)
                    || self.declared_type_names.contains(n);
                if !known {
                    self.type_errors.push(format!(
                        "{}: unknown type `{}` in {}: no type, enum, actor, trait, or alias \
                         of this name is declared; declare it, or add the `use` that brings \
                         it into scope",
                        span.loc(),
                        n,
                        ctx,
                    ));
                    return;
                }
                if !args.is_empty() {
                    let declared = self
                        .generic_types
                        .get(n)
                        .map(|g| g.type_params.len())
                        .or_else(|| self.generic_enums.get(n).map(|g| g.type_params.len()));
                    match declared {
                        Some(k) if k != args.len() => {
                            self.type_errors.push(format!(
                                "{}: `{}` declares {} type parameter(s) but {} type \
                                 argument(s) are supplied in {}",
                                span.loc(),
                                n,
                                k,
                                args.len(),
                                ctx,
                            ));
                        }
                        None => {
                            self.type_errors.push(format!(
                                "{}: type `{}` takes no type arguments but {} are supplied \
                                 in {}",
                                span.loc(),
                                n,
                                args.len(),
                                ctx,
                            ));
                        }
                        _ => {}
                    }
                }
                for a in args {
                    self.check_annotation_ty(a, tparams, span, ctx, nominal);
                }
            }
            Type::Map(k, v) => {
                if let Some(msg) = self.map_key_annotation_error(k, span, ctx) {
                    self.type_errors.push(msg);
                }
                self.check_annotation_ty(k, tparams, span, ctx, nominal);
                self.check_annotation_ty(v, tparams, span, ctx, nominal);
            }
            Type::Vec(inner)
            | Type::Array(inner, _)
            | Type::Ptr(inner)
            | Type::Channel(inner)
            | Type::Coroutine(inner)
            | Type::Generator(inner)
            | Type::View(inner)
            | Type::Frozen(inner)
            | Type::Alias(_, inner)
            | Type::Newtype(_, inner) => {
                self.check_annotation_ty(inner, tparams, span, ctx, nominal);
            }
            Type::Tuple(elems) => {
                for e in elems {
                    self.check_annotation_ty(e, tparams, span, ctx, nominal);
                }
            }
            Type::Fn(params, ret) => {
                for p in params {
                    self.check_annotation_ty(p, tparams, span, ctx, nominal);
                }
                self.check_annotation_ty(ret, tparams, span, ctx, nominal);
            }
            _ => {}
        }
    }

    pub(in crate::typer) fn map_key_error_in(
        &self,
        ty: &Type,
        span: ast::Span,
        ctx: &str,
    ) -> Option<String> {
        match ty {
            Type::Map(k, v) => self
                .map_key_annotation_error(k, span, ctx)
                .or_else(|| self.map_key_error_in(k, span, ctx))
                .or_else(|| self.map_key_error_in(v, span, ctx)),
            Type::Vec(inner)
            | Type::Array(inner, _)
            | Type::Ptr(inner)
            | Type::Channel(inner)
            | Type::Coroutine(inner)
            | Type::Generator(inner)
            | Type::View(inner)
            | Type::Frozen(inner)
            | Type::Alias(_, inner)
            | Type::Newtype(_, inner) => self.map_key_error_in(inner, span, ctx),
            Type::Tuple(elems) => elems
                .iter()
                .find_map(|e| self.map_key_error_in(e, span, ctx)),
            Type::Fn(params, ret) => params
                .iter()
                .find_map(|p| self.map_key_error_in(p, span, ctx))
                .or_else(|| self.map_key_error_in(ret, span, ctx)),
            Type::Struct(_, args) => args
                .iter()
                .find_map(|a| self.map_key_error_in(a, span, ctx)),
            _ => None,
        }
    }

    pub(in crate::typer) fn map_key_annotation_error(
        &self,
        key: &Type,
        span: ast::Span,
        ctx: &str,
    ) -> Option<String> {
        match key {
            Type::String | Type::Param(_) | Type::TypeVar(_) => None,
            _ => Some(format!(
                "{}: map keys are strings in the current runtime, so a map with `{}` keys \
                 cannot be constructed ({}); use string keys — `Map of V` is shorthand for \
                 `Map<string, V>` — or encode the key as a string",
                span.loc(),
                key,
                ctx,
            )),
        }
    }
}
