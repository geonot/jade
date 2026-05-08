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
                _ => format!("{:?}", subject_ty),
            };
            return Err(format!(
                "non-exhaustive match on {ty_name}: missing {missing_str}"
            ));
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

        let ty = self.resolve_ty(ty.clone());

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
                            missing.push(vname.as_str());
                        } else {
                            let fields = vec!["_"; field_tys.len()].join(", ");
                            missing.push(format!("{}({})", vname, fields));
                        }
                    } else if !field_tys.is_empty() {
                        for (i, ft) in field_tys.iter().enumerate() {
                            let col: Vec<&hir::Pat> =
                                sub_lists.iter().filter_map(|subs| subs.get(i)).collect();
                            let sub_missing = self.find_missing_patterns(&col, ft);
                            for sm in &sub_missing {
                                let fields: Vec<String> = field_tys
                                    .iter()
                                    .enumerate()
                                    .map(|(j, _)| if j == i { sm.clone() } else { "_".to_string() })
                                    .collect();
                                missing.push(format!("{}({})", vname, fields.join(", ")));
                            }
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
            Type::I64 | Type::F64 | Type::String => {
                vec!["_".to_string()]
            }
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
