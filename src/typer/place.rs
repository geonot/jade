use std::collections::HashMap;

use crate::hir::{self, DefId};
use crate::intern::Symbol;

use super::MoveReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Proj {
    Field(Symbol),
    Elem(Option<i64>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    pub root: DefId,
    pub root_name: Symbol,
    pub proj: Vec<Proj>,
}

impl Place {
    pub(crate) fn var(root: DefId, root_name: Symbol) -> Self {
        Place {
            root,
            root_name,
            proj: Vec::new(),
        }
    }

    pub(crate) fn is_root(&self) -> bool {
        self.proj.is_empty()
    }

    pub(crate) fn render(&self) -> String {
        let mut out = self.root_name.as_str();
        out.push_str(&Self::render_path(&self.proj));
        out
    }

    pub(crate) fn render_proj(&self) -> String {
        Self::render_path(&self.proj)
            .trim_start_matches('.')
            .to_string()
    }

    fn render_path(proj: &[Proj]) -> String {
        let mut out = String::new();
        for p in proj {
            match p {
                Proj::Field(f) => {
                    out.push('.');
                    out.push_str(&f.as_str());
                }
                Proj::Elem(Some(i)) => out.push_str(&format!("[{i}]")),
                Proj::Elem(None) => out.push_str("[_]"),
            }
        }
        out
    }
}

fn proj_overlaps(a: &Proj, b: &Proj) -> bool {
    match (a, b) {
        (Proj::Field(x), Proj::Field(y)) => x == y,
        (Proj::Elem(Some(i)), Proj::Elem(Some(j))) => i == j,
        (Proj::Elem(_), Proj::Elem(_)) => true,
        _ => false,
    }
}

pub(crate) fn paths_overlap(a: &[Proj], b: &[Proj]) -> bool {
    for (pa, pb) in a.iter().zip(b.iter()) {
        if !proj_overlaps(pa, pb) {
            return false;
        }
    }
    true
}

pub(crate) fn path_covers(outer: &[Proj], inner: &[Proj]) -> bool {
    if outer.len() > inner.len() {
        return false;
    }
    outer
        .iter()
        .zip(inner.iter())
        .all(|(o, i)| proj_overlaps(o, i))
}

impl Place {
    pub(crate) fn overlaps(&self, other: &Place) -> bool {
        self.root == other.root && paths_overlap(&self.proj, &other.proj)
    }

    pub(crate) fn covers(&self, other: &Place) -> bool {
        self.root == other.root && path_covers(&self.proj, &other.proj)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MovedEntry {
    pub place: Place,
    pub reason: MoveReason,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MoveSet {
    by_root: HashMap<DefId, Vec<MovedEntry>>,
}

impl MoveSet {
    pub(crate) fn record(&mut self, place: Place, reason: MoveReason) {
        let entries = self.by_root.entry(place.root).or_default();
        if let Some(existing) = entries.iter_mut().find(|e| e.place.proj == place.proj) {
            existing.reason = reason;
        } else {
            entries.push(MovedEntry { place, reason });
        }
    }

    pub(crate) fn whole(&self, root: DefId) -> Option<&MovedEntry> {
        self.by_root
            .get(&root)?
            .iter()
            .find(|e| e.place.proj.is_empty())
    }

    pub(crate) fn covering(&self, place: &Place) -> Option<&MovedEntry> {
        self.by_root
            .get(&place.root)?
            .iter()
            .find(|e| e.place.covers(place))
    }

    pub(crate) fn within(&self, place: &Place) -> Option<&MovedEntry> {
        self.by_root
            .get(&place.root)?
            .iter()
            .find(|e| e.place.proj.len() > place.proj.len() && place.covers(&e.place))
    }

    pub(crate) fn clear_root(&mut self, root: DefId) {
        self.by_root.remove(&root);
    }

    pub(crate) fn clear_place(&mut self, place: &Place) {
        if let Some(entries) = self.by_root.get_mut(&place.root) {
            entries.retain(|e| !place.covers(&e.place));
            if entries.is_empty() {
                self.by_root.remove(&place.root);
            }
        }
    }

    pub(crate) fn roots(&self) -> impl Iterator<Item = (&DefId, &Vec<MovedEntry>)> {
        self.by_root.iter()
    }

    pub(crate) fn entries_for(&self, root: DefId) -> &[MovedEntry] {
        self.by_root.get(&root).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub(crate) fn union(&mut self, other: &MoveSet) {
        for (root, entries) in &other.by_root {
            let mine = self.by_root.entry(*root).or_default();
            for e in entries {
                if let Some(existing) = mine.iter_mut().find(|m| m.place.proj == e.place.proj) {
                    existing.reason = e.reason;
                } else {
                    mine.push(e.clone());
                }
            }
        }
    }
}

pub(crate) fn place_of_expr(e: &hir::Expr) -> Option<Place> {
    match &e.kind {
        hir::ExprKind::Var(id, name) => Some(Place::var(*id, *name)),
        hir::ExprKind::Field(base, fname, _) => {
            let mut p = place_of_expr(base)?;
            p.proj.push(Proj::Field(*fname));
            Some(p)
        }
        hir::ExprKind::Index(base, idx) => {
            let mut p = place_of_expr(base)?;
            p.proj.push(Proj::Elem(const_index(idx)));
            Some(p)
        }
        hir::ExprKind::VecMethod(recv, m, args) | hir::ExprKind::MapMethod(recv, m, args)
            if m.as_str() == "get" && args.len() == 1 =>
        {
            let mut p = place_of_expr(recv)?;
            p.proj.push(Proj::Elem(const_index(&args[0])));
            Some(p)
        }
        hir::ExprKind::Coerce(inner, _) => place_of_expr(inner),
        _ => None,
    }
}

fn const_index(e: &hir::Expr) -> Option<i64> {
    match &e.kind {
        hir::ExprKind::Int(i) => Some(*i),
        hir::ExprKind::Coerce(inner, _) => const_index(inner),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(name: &str) -> Proj {
        Proj::Field(Symbol::intern(name))
    }

    fn place(root: u32, proj: Vec<Proj>) -> Place {
        Place {
            root: DefId(root),
            root_name: Symbol::intern("s"),
            proj,
        }
    }

    #[test]
    fn root_overlaps_everything_under_it() {
        let whole = place(1, vec![]);
        let field = place(1, vec![f("a")]);
        let deep = place(1, vec![f("a"), f("b")]);
        assert!(whole.overlaps(&field));
        assert!(field.overlaps(&whole));
        assert!(whole.covers(&deep));
        assert!(!deep.covers(&whole));
    }

    #[test]
    fn sibling_fields_are_disjoint() {
        let a = place(1, vec![f("a")]);
        let b = place(1, vec![f("b")]);
        assert!(!a.overlaps(&b));
        let deep_a = place(1, vec![f("a"), f("x")]);
        assert!(!deep_a.overlaps(&b));
        assert!(deep_a.overlaps(&a));
    }

    #[test]
    fn different_roots_never_overlap() {
        let a = place(1, vec![]);
        let b = place(2, vec![]);
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn element_overlap_is_index_aware() {
        let e0 = place(1, vec![Proj::Elem(Some(0))]);
        let e1 = place(1, vec![Proj::Elem(Some(1))]);
        let eu = place(1, vec![Proj::Elem(None)]);
        assert!(!e0.overlaps(&e1));
        assert!(e0.overlaps(&eu));
        assert!(eu.overlaps(&e1));
        assert!(e0.overlaps(&e0));
    }

    #[test]
    fn field_vs_element_projections_are_disjoint() {
        let fa = place(1, vec![f("a")]);
        let e0 = place(1, vec![Proj::Elem(Some(0))]);
        assert!(!fa.overlaps(&e0));
    }

    #[test]
    fn render_shapes() {
        assert_eq!(place(1, vec![]).render(), "s");
        assert_eq!(place(1, vec![f("a"), f("b")]).render(), "s.a.b");
        assert_eq!(place(1, vec![Proj::Elem(Some(3))]).render(), "s[3]");
        assert_eq!(place(1, vec![Proj::Elem(None)]).render(), "s[_]");
    }
}
