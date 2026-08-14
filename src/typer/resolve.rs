use std::collections::HashMap;

use crate::ast::{self, Span};
use crate::intern::Symbol;
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
                        discriminant: None,
                        span: s,
                    },
                    ast::Variant {
                        name: "Nothing".into(),
                        fields: vec![],
                        discriminant: None,
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
                        discriminant: None,
                        span: s,
                    },
                    ast::Variant {
                        name: "Err".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("E".into()),
                        }],
                        discriminant: None,
                        span: s,
                    },
                ],
                span: s,
            });

        if !self.enums.contains_key(&Symbol::intern("StoreError")) {
            let sed = ast::ErrDef {
                name: "StoreError".into(),
                variants: ["Duplicate", "Missing", "Constraint", "Io"]
                    .iter()
                    .map(|n| ast::ErrVariant {
                        name: (*n).into(),
                        fields: vec![],
                        span: s,
                    })
                    .collect(),
                span: s,
            };
            self.declare_err_def_sig(&sed);
            self.store_error_def = Some(sed);
        }

        self.traits.entry("Iter".into()).or_insert_with(|| {
            vec![super::TraitMethodSig {
                name: "next".into(),
                _params: vec![],
                _ret: Some(Type::Enum("Option".into())),
                has_default: false,
            }]
        });

        self.traits.entry("From".into()).or_insert_with(|| {
            vec![super::TraitMethodSig {
                name: "from".into(),
                _params: vec![("s".into(), Some(Type::Param("S".into())))],
                _ret: None,
                has_default: false,
            }]
        });
    }

    pub(crate) fn desugar_bang_ret(ret: &Type, error_types: &[Type]) -> Type {
        if error_types.len() != 1 {
            return ret.clone();
        }
        let err_name = match &error_types[0] {
            Type::Enum(n) | Type::Struct(n, _) | Type::Param(n) => *n,
            _ => return ret.clone(),
        };
        let already_result = matches!(
            ret,
            Type::Enum(n) | Type::Struct(n, _) if n.as_str() == "Result"
        );
        if already_result {
            return ret.clone();
        }
        let ok_ty = if matches!(ret, Type::Void) {
            Type::Void
        } else {
            ret.clone()
        };
        Type::Struct("Result".into(), vec![ok_ty, Type::Enum(err_name)])
    }

    pub(crate) fn declare_fn_sig(&mut self, f: &ast::Fn) {
        let ptys: Vec<Type> = f
            .params
            .iter()
            .map(|p| p.ty.clone().unwrap_or_else(|| self.infer_ctx.fresh_var()))
            .collect();
        let ret = if f.name == "main" {
            Type::I32
        } else if let Some(ref explicit) = f.ret {
            explicit.clone()
        } else if !f.error_types.is_empty() {
            Type::Void
        } else {
            self.infer_ctx.fresh_var()
        };

        let ret = if f.is_generator && f.name != "main" {
            Type::Generator(Box::new(ret))
        } else {
            ret
        };
        let ret = Self::desugar_bang_ret(&ret, &f.error_types);
        let id = self.fresh_id();
        if self.debug_types {
            tracing::debug!(
                target: "jinnc::type",
                "sig {} :: ({}) -> {}",
                f.name,
                ptys.iter()
                    .map(|t| format!("{t}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                ret
            );
        }
        self.fns.insert(f.name, (id, ptys, ret));
        self.fn_param_names
            .insert(f.name, f.params.iter().map(|p| p.name.as_str()).collect());
        self.fn_defaults
            .insert(f.name, f.params.iter().map(|p| p.default.clone()).collect());
        self.fn_param_access
            .insert(f.name, f.params.iter().map(|p| p.access_mod).collect());
    }

    pub(crate) fn declare_method_sig_by_ptr(&mut self, type_name: &str, m: &ast::Fn) {
        self.declare_method_sig_impl(type_name, m, true);
    }

    pub(crate) fn from_method_name(type_name: &str, m: &ast::Fn) -> String {
        let src = m
            .params
            .first()
            .and_then(|p| p.ty.as_ref())
            .map(Self::type_name_str)
            .unwrap_or_default();
        format!("{type_name}_from_{src}")
    }

    fn type_name_str(t: &Type) -> String {
        match t {
            Type::Enum(n) | Type::Struct(n, _) | Type::Param(n) => n.as_str(),
            other => format!("{other}"),
        }
    }

    pub(crate) fn declare_static_method_sig(&mut self, type_name: &str, m: &ast::Fn) {
        let method_name: Symbol = Self::from_method_name(type_name, m).into();
        let ptys: Vec<Type> = m
            .params
            .iter()
            .map(|p| p.ty.clone().unwrap_or_else(|| self.infer_ctx.fresh_var()))
            .collect();
        let ret = m.ret.clone().unwrap_or_else(|| {
            if m.error_types.is_empty() {
                self.infer_ctx.fresh_var()
            } else {
                Type::Void
            }
        });
        let ret = Self::desugar_bang_ret(&ret, &m.error_types);
        let id = self.fresh_id();
        self.fns.insert(method_name, (id, ptys, ret));
        let accs: Vec<Option<ast::AccessMod>> = m.params.iter().map(|p| p.access_mod).collect();
        self.fn_param_access.insert(method_name, accs);
        self.fn_param_names.insert(
            method_name,
            m.params.iter().map(|p| p.name.as_str()).collect(),
        );
    }

    fn declare_method_sig_impl(&mut self, type_name: &str, m: &ast::Fn, by_ptr: bool) {
        let method_name: Symbol = format!("{type_name}_{}", m.name).into();
        let self_ty = if by_ptr {
            Type::Ptr(Box::new(Type::Struct(type_name.into(), vec![])))
        } else {
            Type::Struct(type_name.into(), vec![])
        };
        let mut ptys = vec![self_ty];
        for p in &m.params {
            if by_ptr && p.name == "self" {
                continue;
            }
            ptys.push(p.ty.clone().unwrap_or_else(|| self.infer_ctx.fresh_var()));
        }
        let ret = m.ret.clone().unwrap_or_else(|| {
            if m.error_types.is_empty() {
                self.infer_ctx.fresh_var()
            } else {
                Type::Void
            }
        });
        let ret = Self::desugar_bang_ret(&ret, &m.error_types);
        let id = self.fresh_id();
        self.fns.insert(method_name, (id, ptys, ret));

        let mut accs: Vec<Option<ast::AccessMod>> = vec![None];
        for p in &m.params {
            if by_ptr && p.name == "self" {
                continue;
            }
            accs.push(p.access_mod);
        }
        self.fn_param_access.insert(method_name, accs);
    }

    pub(crate) fn declare_type_def(&mut self, td: &ast::TypeDef) {
        let fields: Vec<(Symbol, Type)> = td
            .fields
            .iter()
            .map(|f| {
                let ty = f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f));
                if f.ty.is_none() {
                    self.unannotated_struct_fields.push((
                        td.name.as_str(),
                        f.name.as_str(),
                        ty.clone(),
                        f.span,
                    ));
                }
                (f.name, ty)
            })
            .collect();
        for (fname, fty) in &fields {
            if crate::typer::expr::views::type_contains_view(fty) {
                self.type_errors.push(format!(
                    "field `{}.{}`: a view cannot be stored in a struct field — views \
                     are second-class borrows that never escape; store the owning \
                     container or a copied slice instead",
                    td.name, fname
                ));
            }
        }
        if td.fields.iter().any(|f| f.ty.is_none()) {
            self.inferred_field_structs.insert(td.name);
        }
        self.structs.insert(td.name, fields);
        self.struct_attrs.insert(td.name, td.layout.clone());
    }

    pub(crate) fn check_category_assertion(&mut self, td: &ast::TypeDef) -> Result<(), String> {
        let Some(cat) = self.struct_attrs.get(&td.name).and_then(|a| a.category) else {
            return Ok(());
        };
        let is_agg = self.type_is_aggregate(&Type::Struct(td.name, vec![]));
        match cat {
            ast::CategoryAssert::Value if is_agg => {
                let witness = self
                    .structs
                    .get(&td.name)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .find(|(_, t)| self.type_is_aggregate(t));
                let detail = match witness {
                    Some((n, t)) => format!(
                        ": field `{n}` has the aggregate type `{t}`, so assignments of \
                         `{}` move",
                        td.name
                    ),
                    None => String::from(": it is marked @resource"),
                };
                Err(format!(
                    "{}: `{}` is asserted `@value` but it is an aggregate{}; remove the \
                     aggregate field, or change the assertion to `@aggregate` and update \
                     the callers that relied on copy semantics",
                    td.span.loc(),
                    td.name,
                    detail,
                ))
            }
            ast::CategoryAssert::Aggregate if !is_agg => Err(format!(
                "{}: `{}` is asserted `@aggregate` but every field is a value type, so it \
                 is a value (assignments copy); add the intended aggregate field, or \
                 change the assertion to `@value`",
                td.span.loc(),
                td.name,
            )),
            _ => Ok(()),
        }
    }

    pub(crate) fn declare_enum_def(&mut self, ed: &ast::EnumDef) {
        let mut variants = Vec::new();
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys: Vec<Type> = v.fields.iter().map(|f| f.ty.clone()).collect();
            for fty in &ftys {
                if crate::typer::expr::views::type_contains_view(fty) {
                    self.type_errors.push(format!(
                        "variant `{}:{}`: a view cannot be stored in an enum payload — \
                         views are second-class borrows that never escape",
                        ed.name, v.name
                    ));
                }
            }
            let tag = v.discriminant.map(|d| d as u32).unwrap_or(tag as u32);
            self.variant_tags.insert(v.name, (ed.name, tag));
            variants.push((v.name, ftys));
        }
        self.enums.insert(ed.name, variants);
    }

    pub(crate) fn declare_extern_sig(&mut self, ef: &ast::ExternFn) {
        if self.externs.contains_key(&ef.name) {
            return;
        }
        let ptys: Vec<Type> = ef.params.iter().map(|(_, t)| t.clone()).collect();
        let id = self.fresh_id();

        self.externs.insert(ef.name, (id, ptys, ef.ret.clone()));
    }

    pub(crate) fn declare_err_def_sig(&mut self, ed: &ast::ErrDef) {
        let mut variants = Vec::new();
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys = v.fields.clone();
            self.variant_tags.insert(v.name, (ed.name, tag as u32));
            variants.push((v.name, ftys));
        }
        self.enums.insert(ed.name, variants);
        self.err_enum_names.insert(ed.name);
    }

    pub(crate) fn declare_actor_def(&mut self, ad: &ast::ActorDef) {
        let id = self.fresh_id();
        let fields: Vec<(Symbol, Type)> = ad
            .fields
            .iter()
            .map(|f| {
                (
                    f.name,
                    f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)),
                )
            })
            .collect();
        let mut next_tag: u32 = 0;
        let handlers: Vec<(Symbol, Vec<Type>, u32)> = ad
            .handlers
            .iter()
            .map(|h| {
                let ptys: Vec<Type> = h
                    .params
                    .iter()
                    .map(|p| p.ty.clone().unwrap_or_else(|| self.infer_ctx.fresh_var()))
                    .collect();
                let tag = if h.is_loop {
                    u32::MAX
                } else {
                    let t = next_tag;
                    next_tag = next_tag.saturating_add(1);
                    t
                };
                (h.name, ptys, tag)
            })
            .collect();
        self.actors.insert(ad.name, (id, fields, handlers));
    }

    pub(crate) fn declare_trait_def(&mut self, td: &ast::TraitDef) {
        let sigs: Vec<super::TraitMethodSig> = td
            .methods
            .iter()
            .map(|m| super::TraitMethodSig {
                name: m.name,
                _params: m
                    .params
                    .iter()
                    .map(|p| (p.name.as_str(), p.ty.clone()))
                    .collect(),
                _ret: m.ret.clone(),
                has_default: m.default_body.is_some(),
            })
            .collect();
        self.traits.insert(td.name, sigs);
        self.trait_defs.insert(td.name, td.clone());
        if !td.assoc_types.is_empty() {
            self.trait_assoc_types
                .insert(td.name, td.assoc_types.iter().map(|s| s.as_str()).collect());
        }
    }

    pub(crate) fn declare_impl_block(&mut self, ib: &ast::ImplBlock) -> Result<(), String> {
        if !self.structs.contains_key(&ib.type_name)
            && !self.enums.contains_key(&ib.type_name)
            && !self.err_enum_names.contains(&ib.type_name)
        {
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

            if let Some(required_assocs) = self.trait_assoc_types.get(trait_name) {
                let provided: Vec<String> = ib
                    .assoc_type_bindings
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect();
                for required in required_assocs {
                    if !provided.contains(required) {
                        return Err(format!(
                            "line {}: impl {} for {} is missing required associated type '{}'",
                            ib.span.line, trait_name, ib.type_name, required
                        ));
                    }
                }
            }

            let trait_sigs = self.traits.get(trait_name).cloned().unwrap_or_else(|| {
                panic!(
                    "ICE: trait '{}' not found during impl validation",
                    trait_name
                )
            });
            let impl_method_names: Vec<String> =
                ib.methods.iter().map(|m| m.name.as_str()).collect();
            for sig in &trait_sigs {
                if !sig.has_default && !impl_method_names.contains(&sig.name.as_str()) {
                    return Err(format!(
                        "line {}: impl {} for {} is missing required method '{}'",
                        ib.span.line, trait_name, ib.type_name, sig.name
                    ));
                }
            }

            self.check_impl_signatures(ib, trait_name)?;

            let synthesized =
                self.synthesize_default_methods(*trait_name, ib.type_name, &impl_method_names);
            if !synthesized.is_empty() {
                self.trait_default_methods
                    .insert((ib.type_name, *trait_name), synthesized);
            }

            self.trait_impls
                .entry(ib.type_name)
                .or_default()
                .push(trait_name.as_str());

            if !ib.trait_type_args.is_empty() {
                self.trait_impl_type_args
                    .insert((ib.type_name, *trait_name), ib.trait_type_args.clone());
            }

            for (assoc_name, assoc_ty) in &ib.assoc_type_bindings {
                self.assoc_types
                    .insert((ib.type_name, *assoc_name), assoc_ty.clone());
            }
        }

        let mut methods_filled: Vec<ast::Fn> = ib.methods.clone();
        if let Some(ref trait_name) = ib.trait_name
            && let Some(td) = self.trait_defs.get(trait_name)
        {
            let mut subst: HashMap<Symbol, Type> = HashMap::new();
            for (tp, ta) in td.type_params.iter().zip(ib.trait_type_args.iter()) {
                subst.insert(*tp, ta.clone());
            }
            for (an, at) in &ib.assoc_type_bindings {
                subst.insert(*an, at.clone());
            }
            for m in &mut methods_filled {
                let Some(tm) = td.methods.iter().find(|tm| tm.name == m.name) else {
                    continue;
                };
                for (ip, tp) in m.params.iter_mut().zip(tm.params.iter()).skip(1) {
                    if ip.ty.is_none()
                        && let Some(tt) = &tp.ty
                    {
                        let want =
                            Self::subst_self_ty(Self::substitute_type(tt, &subst), ib.type_name);
                        if !type_is_open(&want) {
                            ip.ty = Some(want);
                        }
                    }
                }
                if m.ret.is_none()
                    && let Some(tr) = &tm.ret
                {
                    let want = Self::subst_self_ty(Self::substitute_type(tr, &subst), ib.type_name);
                    if !type_is_open(&want) {
                        m.ret = Some(want);
                    }
                }
            }
        }

        let is_static_trait = ib.trait_name.map(|t| t.as_str() == "From").unwrap_or(false);
        for m in &methods_filled {
            self.methods
                .entry(ib.type_name)
                .or_default()
                .push(m.clone());
            let takes_self = m.params.first().map(|p| p.name == "self").unwrap_or(false);
            if is_static_trait && !takes_self {
                self.declare_static_method_sig(&ib.type_name.as_str(), m);
            } else {
                self.declare_method_sig_by_ptr(&ib.type_name.as_str(), m);
            }
        }

        if let Some(trait_name) = ib.trait_name
            && let Some(synthesized) = self
                .trait_default_methods
                .get(&(ib.type_name, trait_name))
                .cloned()
        {
            for m in &synthesized {
                self.methods
                    .entry(ib.type_name)
                    .or_default()
                    .push(m.clone());
                self.declare_method_sig_by_ptr(&ib.type_name.as_str(), m);
            }
        }

        Ok(())
    }

    fn synthesize_default_methods(
        &self,
        trait_name: Symbol,
        type_name: Symbol,
        impl_method_names: &[String],
    ) -> Vec<ast::Fn> {
        let Some(td) = self.trait_defs.get(&trait_name) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for tm in &td.methods {
            let Some(body) = &tm.default_body else {
                continue;
            };
            if impl_method_names.contains(&tm.name.as_str()) {
                continue;
            }
            let ret = tm.ret.clone().map(|t| Self::subst_self_ty(t, type_name));
            let params = tm
                .params
                .iter()
                .map(|p| ast::Param {
                    name: p.name,
                    ty: p.ty.clone().map(|t| Self::subst_self_ty(t, type_name)),
                    default: p.default.clone(),
                    literal: p.literal.clone(),
                    access_mod: p.access_mod,
                    span: p.span,
                })
                .collect();
            out.push(ast::Fn {
                name: tm.name,
                type_params: Vec::new(),
                type_bounds: Vec::new(),
                params,
                ret,
                error_types: tm.error_types.clone(),
                needs: None,
                body: body.clone(),
                is_generator: false,
                attrs: ast::FnAttrs::default(),
                span: tm.span,
            });
        }
        out
    }

    fn check_impl_signatures(
        &self,
        ib: &ast::ImplBlock,
        trait_name: &Symbol,
    ) -> Result<(), String> {
        let Some(td) = self.trait_defs.get(trait_name) else {
            return Ok(());
        };
        let mut subst: HashMap<Symbol, Type> = HashMap::new();
        for (tp, ta) in td.type_params.iter().zip(ib.trait_type_args.iter()) {
            subst.insert(*tp, ta.clone());
        }
        for (an, at) in &ib.assoc_type_bindings {
            subst.insert(*an, at.clone());
        }

        let row_names = |row: &[Type]| -> std::collections::BTreeSet<String> {
            row.iter()
                .map(|et| {
                    let et = Self::subst_self_ty(Self::substitute_type(et, &subst), ib.type_name);
                    match et {
                        Type::Enum(n) | Type::Struct(n, _) | Type::Param(n) => n.to_string(),
                        other => format!("{other:?}"),
                    }
                })
                .collect()
        };
        let render_row = |names: &std::collections::BTreeSet<String>| -> String {
            if names.is_empty() {
                "(none)".to_string()
            } else {
                names.iter().cloned().collect::<Vec<_>>().join(" | ")
            }
        };

        for tm in &td.methods {
            let Some(m) = ib.methods.iter().find(|m| m.name == tm.name) else {
                continue;
            };

            let trait_row = row_names(&tm.error_types);
            let impl_row = row_names(&m.error_types);
            if trait_row != impl_row {
                return Err(format!(
                    "{}: method `{}` of impl {} for {} declares error row `{}`, but the \
                     trait declares `{}` at {} — an impl may neither widen nor narrow the \
                     trait's error row",
                    m.span.loc(),
                    m.name,
                    trait_name,
                    ib.type_name,
                    render_row(&impl_row),
                    render_row(&trait_row),
                    tm.span.loc(),
                ));
            }

            if tm.params.len() != m.params.len() {
                return Err(format!(
                    "{}: method `{}` of impl {} for {} takes {} parameter(s), but the trait \
                     declares {} at {}",
                    m.span.loc(),
                    m.name,
                    trait_name,
                    ib.type_name,
                    m.params.len().saturating_sub(1),
                    tm.params.len().saturating_sub(1),
                    tm.span.loc(),
                ));
            }

            let resolve_trait_ty = |t: &Type| -> Type {
                Self::subst_self_ty(Self::substitute_type(t, &subst), ib.type_name)
            };
            let resolve_impl_ty =
                |t: &Type| -> Type { Self::subst_self_ty(t.clone(), ib.type_name) };

            for (tp, ip) in tm.params.iter().zip(m.params.iter()).skip(1) {
                let (Some(tt), Some(it)) = (&tp.ty, &ip.ty) else {
                    continue;
                };
                let want = resolve_trait_ty(tt);
                if type_is_open(&want) {
                    let got = resolve_impl_ty(it);
                    if !open_type_admits(&want, &got) {
                        return Err(format!(
                            "{}: parameter `{}` of method `{}` in impl {} for {} has type \
                             `{}`, which cannot instantiate the trait's declared `{}` at {}",
                            m.span.loc(),
                            ip.name,
                            m.name,
                            trait_name,
                            ib.type_name,
                            got,
                            want,
                            tm.span.loc(),
                        ));
                    }
                    continue;
                }
                let got = resolve_impl_ty(it);
                if want != got {
                    return Err(format!(
                        "{}: parameter `{}` of method `{}` in impl {} for {} has type \
                         `{}`, but the trait declares `{}` at {}",
                        m.span.loc(),
                        ip.name,
                        m.name,
                        trait_name,
                        ib.type_name,
                        got,
                        want,
                        tm.span.loc(),
                    ));
                }
            }

            if let (Some(tr), Some(ir)) = (&tm.ret, &m.ret) {
                let want = resolve_trait_ty(tr);
                let got = resolve_impl_ty(ir);
                if type_is_open(&want) {
                    if !open_type_admits(&want, &got) {
                        return Err(format!(
                            "{}: method `{}` of impl {} for {} returns `{}`, which cannot \
                             instantiate the trait's declared `{}` at {}",
                            m.span.loc(),
                            m.name,
                            trait_name,
                            ib.type_name,
                            got,
                            want,
                            tm.span.loc(),
                        ));
                    }
                } else if want != got {
                    return Err(format!(
                        "{}: method `{}` of impl {} for {} returns `{}`, but the trait \
                         declares `{}` at {}",
                        m.span.loc(),
                        m.name,
                        trait_name,
                        ib.type_name,
                        got,
                        want,
                        tm.span.loc(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn subst_self_ty(ty: Type, type_name: Symbol) -> Type {
        match ty {
            Type::Param(n) if n.as_str() == "Self" => Type::Struct(type_name, vec![]),
            Type::Struct(n, _) if n.as_str() == "Self" => Type::Struct(type_name, vec![]),
            Type::Ptr(inner) => Type::Ptr(Box::new(Self::subst_self_ty(*inner, type_name))),
            other => other,
        }
    }

    pub(crate) fn infer_param_types(&mut self, _prog: &ast::Program) {
        let struct_names: Vec<Symbol> = self.structs.keys().cloned().collect();
        let fn_keys: Vec<Symbol> = self.fns.keys().cloned().collect();
        for fname in &fn_keys {
            for sname in &struct_names {
                let prefix = format!("{}_", sname);
                if fname.starts_with(&prefix)
                    && fname.len() > prefix.len()
                    && let Some((_, ptys, _)) = self.fns.get(fname)
                    && let Some(Type::TypeVar(_)) = ptys.first()
                    && let Some(ast_fn) = self.inferable_fns.get(fname)
                    && ast_fn.params.first().is_some_and(|p| p.name == "self")
                {
                    let self_ty = Type::Struct(*sname, vec![]);
                    let tv = ptys[0].clone();
                    let _ = self.infer_ctx.unify(&tv, &self_ty);
                }
            }
        }

        let keys: Vec<Symbol> = self.fns.keys().cloned().collect();
        for k in keys {
            let entry = self.fns.get_mut(&k).unwrap();
            for ty in &mut entry.1 {
                if matches!(ty, Type::TypeVar(_)) {
                    *ty = self.infer_ctx.shallow_resolve(ty);
                }
            }
            if entry.2.has_type_var() {
                entry.2 = self.infer_ctx.shallow_resolve(&entry.2);
            }
        }

        if self.debug_types {
            tracing::debug!(target: "jinnc::type", "resolved final signatures:");
            let mut names: Vec<Symbol> = self.fns.keys().cloned().collect();
            names.sort();
            for name in &names {
                let (_, ptys, ret) = &self.fns[name];
                tracing::debug!(
                    target: "jinnc::type",
                    "  {} :: ({}) -> {}",
                    name,
                    ptys.iter()
                        .map(|t| format!("{t}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    ret
                );
            }
        }
    }
}

fn open_type_admits(want: &Type, got: &Type) -> bool {
    fn freshen(
        t: &Type,
        ctx: &mut super::unify::InferCtx,
        map: &mut HashMap<Symbol, Type>,
    ) -> Type {
        match t {
            Type::Param(n) => map.entry(*n).or_insert_with(|| ctx.fresh_var()).clone(),
            Type::Array(i, n) => Type::Array(Box::new(freshen(i, ctx, map)), *n),
            Type::Vec(i) => Type::Vec(Box::new(freshen(i, ctx, map))),
            Type::Ptr(i) => Type::Ptr(Box::new(freshen(i, ctx, map))),
            Type::Channel(i) => Type::Channel(Box::new(freshen(i, ctx, map))),
            Type::Coroutine(i) => Type::Coroutine(Box::new(freshen(i, ctx, map))),
            Type::Generator(i) => Type::Generator(Box::new(freshen(i, ctx, map))),
            Type::View(i) => Type::View(Box::new(freshen(i, ctx, map))),
            Type::Frozen(i) => Type::Frozen(Box::new(freshen(i, ctx, map))),
            Type::Alias(n, i) => Type::Alias(*n, Box::new(freshen(i, ctx, map))),
            Type::Newtype(n, i) => Type::Newtype(*n, Box::new(freshen(i, ctx, map))),
            Type::Map(k, v) => Type::Map(
                Box::new(freshen(k, ctx, map)),
                Box::new(freshen(v, ctx, map)),
            ),
            Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| freshen(t, ctx, map)).collect()),
            Type::Fn(ps, r) => Type::Fn(
                ps.iter().map(|t| freshen(t, ctx, map)).collect(),
                Box::new(freshen(r, ctx, map)),
            ),
            Type::Struct(n, args) => {
                Type::Struct(*n, args.iter().map(|t| freshen(t, ctx, map)).collect())
            }
            other => other.clone(),
        }
    }
    let mut scratch = super::unify::InferCtx::new();
    let mut map = HashMap::new();
    let want_f = freshen(want, &mut scratch, &mut map);
    scratch.unify(&want_f, got).is_ok()
}

fn type_is_open(t: &Type) -> bool {
    match t {
        Type::Param(_) | Type::TypeVar(_) => true,
        Type::Array(i, _)
        | Type::Vec(i)
        | Type::Ptr(i)
        | Type::Channel(i)
        | Type::Coroutine(i)
        | Type::Generator(i)
        | Type::Alias(_, i)
        | Type::Newtype(_, i) => type_is_open(i),
        Type::Map(k, v) => type_is_open(k) || type_is_open(v),
        Type::Tuple(ts) => ts.iter().any(type_is_open),
        Type::Fn(ps, r) => ps.iter().any(type_is_open) || type_is_open(r),
        Type::Struct(_, args) => args.iter().any(type_is_open),
        _ => false,
    }
}
