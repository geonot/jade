use super::super::Typer;
use crate::ast::Span;
use crate::hir;
use crate::types::Type;

impl Typer {
    pub(in crate::typer) fn check_exhaustiveness(
        &self,
        subject_ty: &Type,
        arms: &[hir::Arm],
        _span: Span,
    ) -> Result<(), String> {
        let pats: Vec<&hir::Pat> = arms
            .iter()
            .filter(|a| a.guard.is_none())
            .map(|a| &a.pat)
            .collect();

        let missing = self.find_missing_patterns(&pats, subject_ty);
        if !missing.is_empty() {
            let missing_str = missing.join(", ");
            let ty_name = match subject_ty {
                Type::Enum(n) => format!("`{n}`"),
                Type::Bool => "Bool".to_string(),
                other => format!("`{other}`"),
            };
            return Err(format!(
                "non-exhaustive match on {ty_name}: missing {missing_str}"
            ));
        }

        let mut seen_lits: std::collections::HashSet<String> = std::collections::HashSet::new();
        for arm in arms.iter().filter(|a| a.guard.is_none()) {
            if let hir::Pat::Lit(e) = &arm.pat {
                let (key, shown) = match &e.kind {
                    hir::ExprKind::Int(n) => (format!("i{n}"), n.to_string()),
                    hir::ExprKind::Bool(b) => (format!("b{b}"), b.to_string()),
                    hir::ExprKind::Float(f) => (format!("f{:x}", f.to_bits()), f.to_string()),
                    hir::ExprKind::Str(s) => (format!("s{s}"), format!("'{s}'")),
                    _ => continue,
                };
                if !seen_lits.insert(key) {
                    return Err(format!(
                        "duplicate match arm: `{shown}` is matched by an earlier arm and \
                         can never run"
                    ));
                }
            }
        }

        if let Type::Enum(_) = subject_ty {
            let mut seen: Vec<&str> = Vec::new();
            for arm in arms {
                if let hir::Pat::Ctor(n, _, subs, _) = &arm.pat {
                    if subs.is_empty() && seen.contains(&n.as_str()) {
                        eprintln!("warning: unreachable pattern `{n}` — already matched above");
                    }
                    if subs.is_empty() {
                        seen.push(n.as_str());
                    }
                }
            }
        }

        Ok(())
    }

    pub(in crate::typer) fn find_missing_patterns(
        &self,
        pats: &[&hir::Pat],
        ty: &Type,
    ) -> Vec<String> {
        let mut flat: Vec<&hir::Pat> = Vec::new();
        for p in pats {
            Self::flatten_or_pat(p, &mut flat);
        }

        if flat
            .iter()
            .any(|p| matches!(p, hir::Pat::Wild(_) | hir::Pat::Bind(..)))
        {
            return vec![];
        }

        let mut ty = self.resolve_ty(ty.clone());
        while let Type::Alias(_, inner) | Type::Newtype(_, inner) = ty {
            ty = self.resolve_ty((*inner).clone());
        }

        match &ty {
            Type::Enum(name) => {
                let variants = match self.enums.get(name) {
                    Some(v) => v,
                    None => return vec![],
                };
                let mut missing = Vec::new();
                for (vname, field_tys) in variants {
                    let sub_lists: Vec<&Vec<hir::Pat>> = flat
                        .iter()
                        .filter_map(|p| match p {
                            hir::Pat::Ctor(n, _, subs, _) if vname == n => Some(subs),
                            _ => None,
                        })
                        .collect();

                    if sub_lists.is_empty() {
                        if field_tys.is_empty() {
                            missing.push(vname.as_str().to_string());
                        } else {
                            let fields = vec!["_"; field_tys.len()].join(", ");
                            missing.push(format!("{}({})", vname, fields));
                        }
                    } else if field_tys.len() == 1 {
                        let col: Vec<&hir::Pat> =
                            sub_lists.iter().filter_map(|subs| subs.first()).collect();
                        for sm in self.find_missing_patterns(&col, &field_tys[0]) {
                            missing.push(format!("{}({})", vname, sm));
                        }
                    } else if field_tys.len() > 1 {
                        let has_irrefutable = sub_lists.iter().any(|subs| {
                            subs.iter()
                                .all(|p| matches!(p, hir::Pat::Wild(_) | hir::Pat::Bind(..)))
                        });
                        if !has_irrefutable {
                            let fields = vec!["_"; field_tys.len()].join(", ");
                            missing.push(format!("{}({})", vname, fields));
                        }
                    }
                }
                missing
            }
            Type::Bool => {
                let has_true = flat.iter().any(|p| match p {
                    hir::Pat::Lit(e) => matches!(e.kind, hir::ExprKind::Bool(true)),
                    _ => false,
                });
                let has_false = flat.iter().any(|p| match p {
                    hir::Pat::Lit(e) => matches!(e.kind, hir::ExprKind::Bool(false)),
                    _ => false,
                });
                let mut missing = Vec::new();
                if !has_true {
                    missing.push("true".to_string());
                }
                if !has_false {
                    missing.push("false".to_string());
                }
                missing
            }
            Type::Tuple(elem_tys) => {
                let rows: Vec<&Vec<hir::Pat>> = flat
                    .iter()
                    .filter_map(|p| match p {
                        hir::Pat::Tuple(subs, _) => Some(subs),
                        _ => None,
                    })
                    .collect();
                let has_irrefutable = rows.iter().any(|subs| {
                    subs.iter()
                        .all(|p| matches!(p, hir::Pat::Wild(_) | hir::Pat::Bind(..)))
                });
                if has_irrefutable {
                    vec![]
                } else {
                    vec![format!("({})", vec!["_"; elem_tys.len().max(1)].join(", "))]
                }
            }
            Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::F32
            | Type::F64
            | Type::String => vec!["_".to_string()],
            _ => vec![],
        }
    }

    pub(in crate::typer) fn flatten_or_pat<'a>(pat: &'a hir::Pat, out: &mut Vec<&'a hir::Pat>) {
        match pat {
            hir::Pat::Or(pats, _) => {
                for p in pats {
                    Self::flatten_or_pat(p, out);
                }
            }
            _ => out.push(pat),
        }
    }
}
