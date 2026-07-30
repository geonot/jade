use crate::intern::Symbol;
use crate::pkg::SemVer;
use blake3::Hasher;
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopePath(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PkgId(u32);

/// A package's self-declared visibility ceiling (scope.md §4). Declared in the
/// package's *own* manifest; the resolver reads it on the target of a path
/// import, never on the consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    /// Reachable by any consumer via path import (the default — reach-in is
    /// open unless the target opts into a tighter ceiling).
    #[default]
    Public,
    /// Reachable only within the package's own owner-scope subtree: its parent
    /// scope and that scope's descendants. A reach-in from a grandparent,
    /// sibling, or external package is a hard error.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRecord {
    pub name: Symbol,
    pub owner_scope: ScopePath,
    pub version: SemVer,
    pub semantic_hash: [u8; 32],
    pub visibility: Visibility,
}

struct Tables {
    scope_segments: Vec<Vec<Symbol>>,
    scope_lookup: HashMap<Vec<Symbol>, ScopePath>,
    pkgs: Vec<PackageRecord>,
    pkg_lookup: HashMap<(Symbol, ScopePath, String), PkgId>,
}

impl Tables {
    fn new() -> Self {
        let mut t = Tables {
            scope_segments: Vec::new(),
            scope_lookup: HashMap::new(),
            pkgs: Vec::new(),
            pkg_lookup: HashMap::new(),
        };
        let root = t.scope_segments.len() as u32;
        t.scope_segments.push(Vec::new());
        t.scope_lookup.insert(Vec::new(), ScopePath(root));
        t
    }
}

thread_local! {
    static TABLES: RefCell<Tables> = RefCell::new(Tables::new());
}

impl ScopePath {
    pub fn root() -> Self {
        ScopePath(0)
    }

    pub fn intern(segments: &[Symbol]) -> Self {
        TABLES.with(|t| {
            let mut t = t.borrow_mut();
            if let Some(sp) = t.scope_lookup.get(segments) {
                return *sp;
            }
            let id = ScopePath(t.scope_segments.len() as u32);
            t.scope_segments.push(segments.to_vec());
            t.scope_lookup.insert(segments.to_vec(), id);
            id
        })
    }

    pub fn child(self, name: Symbol) -> Self {
        TABLES.with(|t| {
            let mut segs = t.borrow().scope_segments[self.0 as usize].clone();
            segs.push(name);
            ScopePath::intern_owned(segs)
        })
    }

    fn intern_owned(segments: Vec<Symbol>) -> Self {
        TABLES.with(|t| {
            let mut t = t.borrow_mut();
            if let Some(sp) = t.scope_lookup.get(&segments) {
                return *sp;
            }
            let id = ScopePath(t.scope_segments.len() as u32);
            t.scope_segments.push(segments.clone());
            t.scope_lookup.insert(segments, id);
            id
        })
    }

    pub fn segments(self) -> Vec<Symbol> {
        TABLES.with(|t| t.borrow().scope_segments[self.0 as usize].clone())
    }

    pub fn is_root(self) -> bool {
        self.0 == 0
    }

    /// True when `self` is an ancestor-or-equal of `other`: every segment of
    /// `self`, in order, is a prefix of `other`'s segments. The root scope is a
    /// prefix of everything. Used for the visibility-ceiling subtree test
    /// (scope.md §4.1).
    pub fn is_prefix_of(self, other: ScopePath) -> bool {
        if self == other {
            return true;
        }
        TABLES.with(|t| {
            let t = t.borrow();
            let a = &t.scope_segments[self.0 as usize];
            let b = &t.scope_segments[other.0 as usize];
            a.len() <= b.len() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
        })
    }

    pub fn render(self) -> String {
        Symbol::join_vec(&self.segments(), ":")
    }
}

impl std::fmt::Display for ScopePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.render())
    }
}

impl PkgId {
    pub fn intern(record: PackageRecord) -> Self {
        TABLES.with(|t| {
            let mut t = t.borrow_mut();
            let key = (record.name, record.owner_scope, record.version.to_string());
            if let Some(id) = t.pkg_lookup.get(&key) {
                return *id;
            }
            let id = PkgId(t.pkgs.len() as u32);
            t.pkgs.push(record);
            t.pkg_lookup.insert(key, id);
            id
        })
    }

    pub fn root(name: Symbol) -> Self {
        PkgId::intern(PackageRecord {
            name,
            owner_scope: ScopePath::root(),
            version: SemVer {
                major: 0,
                minor: 0,
                patch: 0,
            },
            semantic_hash: [0u8; 32],
            visibility: Visibility::Public,
        })
    }

    pub fn record(self) -> PackageRecord {
        TABLES.with(|t| t.borrow().pkgs[self.0 as usize].clone())
    }

    pub fn name(self) -> Symbol {
        TABLES.with(|t| t.borrow().pkgs[self.0 as usize].name)
    }

    pub fn owner_scope(self) -> ScopePath {
        TABLES.with(|t| t.borrow().pkgs[self.0 as usize].owner_scope)
    }

    pub fn visibility(self) -> Visibility {
        TABLES.with(|t| t.borrow().pkgs[self.0 as usize].visibility)
    }

    pub fn fully_qualified(self) -> String {
        let r = self.record();
        if r.owner_scope.is_root() {
            r.name.to_string()
        } else {
            format!("{}:{}", r.owner_scope.render(), r.name)
        }
    }

    pub fn mangle_prefix(self) -> String {
        let r = self.record();
        let h: String = r.semantic_hash[..4]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        h
    }
}

impl std::fmt::Display for PkgId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let r = self.record();
        write!(f, "{}@{}", self.fully_qualified(), r.version)
    }
}

/// Compute the `semantic_hash` for a package.
///
/// Inputs:
///   - `name`: the package's own name
///   - `scope`: the owner scope (dotted path)
///   - `version`: the resolved version
///   - `source_bytes`: concatenated sorted source file contents for this package
///   - `dep_hashes`: semantic hashes of direct dependencies, **sorted ascending**
///     before passing in (caller responsibility — determinism requires a stable order)
///
/// The hash is a Blake3 Merkle step: Hash(domain_sep ++ name ++ version ++ scope
/// ++ source_bytes ++ sorted(dep_hash)*).  Changing any input flips the hash;
/// equal inputs always yield the same hash.
/// For each package A with a `use B` in its manifest, record the scoped PkgId
/// that `B` resolves to from A's perspective.
///
/// Key: `(consumer_pkg_id, dep_name_symbol)`
/// Value: the dep's `PkgId` as seen by the consumer (path-scoped under the
///        consumer's owner scope, per scope.md §2.1).
pub type ScopedUseMap = HashMap<(PkgId, Symbol), PkgId>;

/// Failure of a path-scoped `use` resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseResolveError {
    /// `use <name>` had no matching `requires` entry in the consumer's manifest.
    /// Carries the consumer's fully-qualified path and the unresolved name so
    /// the diagnostic can point at the right `project.jn`.
    Unresolved { consumer: String, name: Symbol },
    /// A path import `use a/b/...` could not resolve one hop: `via` (the
    /// intermediary's fully-qualified path) does not declare `requires next`.
    UnresolvedHop {
        consumer: String,
        via: String,
        next: Symbol,
    },
    /// A path import reached a target declared `visibility internal` from
    /// outside the target's owner-scope subtree (scope.md §4.1).
    VisibilityCeiling {
        consumer: String,
        target: String,
        target_scope: String,
    },
}

impl std::fmt::Display for UseResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UseResolveError::Unresolved { consumer, name } => write!(
                f,
                "unresolved import 'use {name}' in package '{consumer}': add '{name}' to its \
                 project.jn requires"
            ),
            UseResolveError::UnresolvedHop {
                consumer,
                via,
                next,
            } => write!(
                f,
                "path import in package '{consumer}' cannot reach '{next}': package '{via}' does \
                 not declare 'requires {next}' in its project.jn"
            ),
            UseResolveError::VisibilityCeiling {
                consumer,
                target,
                target_scope,
            } => write!(
                f,
                "package '{target}' is declared 'visibility internal' and is reachable only from \
                 within its scope '{target_scope}'; package '{consumer}' is outside that subtree. \
                 Relax the ceiling in '{target}'s project.jn, or import it from within \
                 '{target_scope}'"
            ),
        }
    }
}

/// Resolve a plain `use <name>` against the consumer's own manifest, per
/// scope.md §2.1 ("local, deterministic, no global arbitration").
///
/// The lookup is keyed strictly by `(consumer, name)`: the same `name` resolves
/// to a *different* `PkgId` for a different consumer, and there is **no global
/// fallback** — a name absent from the consumer's `requires` is a hard error,
/// never silently borrowed from another scope.
pub fn resolve_use(
    map: &ScopedUseMap,
    consumer: PkgId,
    name: Symbol,
) -> Result<PkgId, UseResolveError> {
    map.get(&(consumer, name))
        .copied()
        .ok_or_else(|| UseResolveError::Unresolved {
            consumer: consumer.fully_qualified(),
            name,
        })
}

/// Resolve a path import `use seg0/seg1/.../segN` from `consumer`, hop by hop,
/// then enforce the final target's visibility ceiling (scope.md §4).
///
/// Each hop resolves `seg_{i+1}` against the *current* intermediary's manifest
/// (keyed `(current_pkg_id, seg_{i+1})` in the scoped map), so the parent-scoped
/// nesting of scope.md §2.1 is preserved at every step. A single-segment path
/// degrades to `resolve_use`.
///
/// The ceiling check (§4.1) applies only to the final target: the import is
/// legal iff the target is `Public` or the consumer's owner scope is within the
/// target's owner-scope subtree.
pub fn resolve_path_use(
    map: &ScopedUseMap,
    consumer: PkgId,
    path: &[Symbol],
) -> Result<PkgId, UseResolveError> {
    let mut current = consumer;
    for (i, seg) in path.iter().enumerate() {
        match map.get(&(current, *seg)).copied() {
            Some(next) => current = next,
            None => {
                return Err(if i == 0 {
                    UseResolveError::Unresolved {
                        consumer: consumer.fully_qualified(),
                        name: *seg,
                    }
                } else {
                    UseResolveError::UnresolvedHop {
                        consumer: consumer.fully_qualified(),
                        via: current.fully_qualified(),
                        next: *seg,
                    }
                });
            }
        }
    }

    if matches!(current.visibility(), Visibility::Internal) {
        // The consumer's own home scope is its owner_scope extended by its name.
        let consumer_scope = consumer.owner_scope().child(consumer.name());
        let target_scope = current.owner_scope();
        if !target_scope.is_prefix_of(consumer_scope) {
            return Err(UseResolveError::VisibilityCeiling {
                consumer: consumer.fully_qualified(),
                target: current.fully_qualified(),
                target_scope: target_scope.render(),
            });
        }
    }

    Ok(current)
}

/// Build the exact per-item owning-`PkgId` map (scope.md §1.1) that replaces the
/// legacy `prefix_module` string-identity model.
///
/// Module flattening renames a dependency module `m`'s items to `m_<name>`.
/// Given `dep_pkgs` (module symbol → its `PkgId`) and the flattened top-level
/// `item_names`, this attributes each item to the dependency whose module prefix
/// it carries — **once, here** — so downstream queries are an exact lookup rather
/// than a per-call prefix scan. Items matching no dependency prefix are omitted
/// (they belong to the root package and fall through to it). When two module
/// prefixes are both prefixes of a name (e.g. `a` and `a_b`), the longest match
/// wins, so `a_b`'s items attribute to `a_b`, not `a`.
pub fn build_item_pkgs<'a>(
    dep_pkgs: &HashMap<Symbol, PkgId>,
    item_names: impl Iterator<Item = &'a Symbol>,
) -> HashMap<Symbol, PkgId> {
    if dep_pkgs.is_empty() {
        return HashMap::new();
    }
    let mut prefixes: Vec<(String, PkgId)> = dep_pkgs
        .iter()
        .map(|(m, id)| (format!("{}_", m.as_str()), *id))
        .collect();
    prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    let mut out = HashMap::new();
    for name in item_names {
        let n = name.as_str();
        if let Some((_, id)) = prefixes.iter().find(|(p, _)| n.starts_with(p.as_str())) {
            out.insert(*name, *id);
        }
    }
    out
}

pub fn compute_semantic_hash(
    name: Symbol,
    scope: ScopePath,
    version: &SemVer,
    source_bytes: &[u8],
    dep_hashes: &[[u8; 32]],
) -> [u8; 32] {
    let mut h = Hasher::new_derive_key("jinn:pkgid:semantic_hash:v1");
    h.update(name.as_str().as_bytes());
    h.update(&[0u8]);
    let scope_str = scope.render();
    h.update(scope_str.as_bytes());
    h.update(&[0u8]);
    let ver_str = version.to_string();
    h.update(ver_str.as_bytes());
    h.update(&[0u8]);
    let src_len = (source_bytes.len() as u64).to_le_bytes();
    h.update(&src_len);
    h.update(source_bytes);
    let dep_count = (dep_hashes.len() as u64).to_le_bytes();
    h.update(&dep_count);
    for dh in dep_hashes {
        h.update(dh);
    }
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(s: &str) -> Symbol {
        Symbol::intern(s)
    }

    fn ver(maj: u32) -> SemVer {
        SemVer {
            major: maj,
            minor: 0,
            patch: 0,
        }
    }

    #[test]
    fn root_scope_is_empty() {
        let r = ScopePath::root();
        assert!(r.is_root());
        assert_eq!(r.render(), "");
    }

    #[test]
    fn scope_interning_is_stable() {
        let a = ScopePath::intern(&[sym("foo"), sym("baz")]);
        let b = ScopePath::intern(&[sym("foo"), sym("baz")]);
        assert_eq!(a, b);
        assert_eq!(a.render(), "foo:baz");
    }

    #[test]
    fn child_extends_scope() {
        let foo = ScopePath::intern(&[sym("foo")]);
        let foobaz = foo.child(sym("baz"));
        assert_eq!(foobaz.render(), "foo:baz");
        assert_eq!(foobaz.segments().len(), 2);
    }

    #[test]
    fn pkgid_dedups_same_identity() {
        let rec = PackageRecord {
            name: sym("bar"),
            owner_scope: ScopePath::intern(&[sym("foo"), sym("baz")]),
            version: ver(1),
            semantic_hash: [7u8; 32],
            visibility: Visibility::Public,
        };
        let a = PkgId::intern(rec.clone());
        let b = PkgId::intern(rec);
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_versions_are_distinct_ids() {
        let scope = ScopePath::root();
        let v1 = PkgId::intern(PackageRecord {
            name: sym("foo"),
            owner_scope: scope,
            version: ver(1),
            semantic_hash: [0u8; 32],
            visibility: Visibility::Public,
        });
        let v2 = PkgId::intern(PackageRecord {
            name: sym("foo"),
            owner_scope: scope,
            version: ver(2),
            semantic_hash: [0u8; 32],
            visibility: Visibility::Public,
        });
        assert_ne!(v1, v2);
    }

    #[test]
    fn fully_qualified_path_scoping() {
        let scoped = PkgId::intern(PackageRecord {
            name: sym("bar"),
            owner_scope: ScopePath::intern(&[sym("foo"), sym("baz")]),
            version: ver(1),
            semantic_hash: [0u8; 32],
            visibility: Visibility::Public,
        });
        assert_eq!(scoped.fully_qualified(), "foo:baz:bar");
        let root = PkgId::root(sym("bar"));
        assert_eq!(root.fully_qualified(), "bar");
    }

    #[test]
    fn mangle_prefix_from_hash() {
        let p = PkgId::intern(PackageRecord {
            name: sym("x"),
            owner_scope: ScopePath::root(),
            version: ver(1),
            semantic_hash: [
                0xab, 0xcd, 0xef, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0,
            ],
            visibility: Visibility::Public,
        });
        assert_eq!(p.mangle_prefix(), "abcdef01");
    }

    #[test]
    fn semantic_hash_is_deterministic() {
        let scope = ScopePath::root();
        let version = ver(1);
        let src = b"fn main\n  42";
        let h1 = compute_semantic_hash(sym("foo"), scope, &version, src, &[]);
        let h2 = compute_semantic_hash(sym("foo"), scope, &version, src, &[]);
        assert_eq!(h1, h2);
    }

    #[test]
    fn semantic_hash_differs_on_name_change() {
        let scope = ScopePath::root();
        let version = ver(1);
        let src = b"fn main\n  42";
        let h1 = compute_semantic_hash(sym("foo"), scope, &version, src, &[]);
        let h2 = compute_semantic_hash(sym("bar"), scope, &version, src, &[]);
        assert_ne!(h1, h2);
    }

    #[test]
    fn semantic_hash_differs_on_source_change() {
        let scope = ScopePath::root();
        let version = ver(1);
        let h1 = compute_semantic_hash(sym("foo"), scope, &version, b"fn main\n  42", &[]);
        let h2 = compute_semantic_hash(sym("foo"), scope, &version, b"fn main\n  43", &[]);
        assert_ne!(h1, h2);
    }

    #[test]
    fn semantic_hash_differs_on_version_change() {
        let scope = ScopePath::root();
        let src = b"fn main\n  42";
        let h1 = compute_semantic_hash(sym("foo"), scope, &ver(1), src, &[]);
        let h2 = compute_semantic_hash(sym("foo"), scope, &ver(2), src, &[]);
        assert_ne!(h1, h2);
    }

    #[test]
    fn semantic_hash_differs_on_scope_change() {
        let scope_a = ScopePath::intern(&[sym("parent_a")]);
        let scope_b = ScopePath::intern(&[sym("parent_b")]);
        let version = ver(1);
        let src = b"fn main\n  42";
        let h1 = compute_semantic_hash(sym("foo"), scope_a, &version, src, &[]);
        let h2 = compute_semantic_hash(sym("foo"), scope_b, &version, src, &[]);
        assert_ne!(h1, h2);
    }

    #[test]
    fn semantic_hash_includes_dep_hashes() {
        let scope = ScopePath::root();
        let version = ver(1);
        let src = b"fn main\n  42";
        let dep: [u8; 32] = [0xde; 32];
        let h_no_dep = compute_semantic_hash(sym("foo"), scope, &version, src, &[]);
        let h_with_dep = compute_semantic_hash(sym("foo"), scope, &version, src, &[dep]);
        assert_ne!(h_no_dep, h_with_dep);
    }

    #[test]
    fn semantic_hash_dep_order_matters() {
        let scope = ScopePath::root();
        let version = ver(1);
        let src = b"fn main\n  42";
        let d1 = [0x11u8; 32];
        let d2 = [0x22u8; 32];
        let h1 = compute_semantic_hash(sym("foo"), scope, &version, src, &[d1, d2]);
        let h2 = compute_semantic_hash(sym("foo"), scope, &version, src, &[d2, d1]);
        assert_ne!(h1, h2);
    }

    fn scoped(name: &str, scope: &[Symbol]) -> PkgId {
        PkgId::intern(PackageRecord {
            name: sym(name),
            owner_scope: ScopePath::intern(scope),
            version: ver(1),
            semantic_hash: [name.len() as u8; 32],
            visibility: Visibility::Public,
        })
    }

    fn scoped_vis(name: &str, scope: &[Symbol], vis: Visibility) -> PkgId {
        PkgId::intern(PackageRecord {
            name: sym(name),
            owner_scope: ScopePath::intern(scope),
            version: ver(1),
            semantic_hash: [name.len() as u8; 32],
            visibility: vis,
        })
    }

    #[test]
    fn scope_prefix_check() {
        let foo = ScopePath::intern(&[sym("foo")]);
        let foobaz = ScopePath::intern(&[sym("foo"), sym("baz")]);
        let foobar = ScopePath::intern(&[sym("foo"), sym("bar")]);
        assert!(ScopePath::root().is_prefix_of(foobaz));
        assert!(foo.is_prefix_of(foobaz));
        assert!(foobaz.is_prefix_of(foobaz));
        assert!(!foobaz.is_prefix_of(foo));
        assert!(!foobaz.is_prefix_of(foobar));
    }

    #[test]
    fn path_use_resolves_two_hops() {
        // foo (root) -> baz ; baz -> bar.  foo's `use baz/bar` resolves to foo:baz:bar.
        let foo = PkgId::root(sym("foo"));
        let baz = scoped("baz", &[sym("foo")]);
        let bar = scoped("bar", &[sym("foo"), sym("baz")]);
        let mut map = ScopedUseMap::new();
        map.insert((foo, sym("baz")), baz);
        map.insert((baz, sym("bar")), bar);
        let got = resolve_path_use(&map, foo, &[sym("baz"), sym("bar")]).unwrap();
        assert_eq!(got, bar);
        assert_eq!(got.fully_qualified(), "foo:baz:bar");
    }

    #[test]
    fn path_use_missing_hop_errors() {
        let foo = PkgId::root(sym("foo"));
        let baz = scoped("baz", &[sym("foo")]);
        let mut map = ScopedUseMap::new();
        map.insert((foo, sym("baz")), baz);
        // baz never declares `requires qux`.
        let err = resolve_path_use(&map, foo, &[sym("baz"), sym("qux")]).unwrap_err();
        match err {
            UseResolveError::UnresolvedHop { via, next, .. } => {
                assert_eq!(via, "foo:baz");
                assert_eq!(next, sym("qux"));
            }
            other => panic!("expected UnresolvedHop, got {other:?}"),
        }
    }

    #[test]
    fn public_reach_in_allowed_from_grandparent() {
        // bar is public (default): foo (the grandparent) may reach foo:baz:bar.
        let foo = PkgId::root(sym("foo"));
        let baz = scoped("baz", &[sym("foo")]);
        let bar = scoped_vis("bar", &[sym("foo"), sym("baz")], Visibility::Public);
        let mut map = ScopedUseMap::new();
        map.insert((foo, sym("baz")), baz);
        map.insert((baz, sym("bar")), bar);
        assert!(resolve_path_use(&map, foo, &[sym("baz"), sym("bar")]).is_ok());
    }

    #[test]
    fn internal_reach_in_rejected_from_grandparent() {
        // bar is internal to foo:baz: foo (grandparent, scope `foo`) is outside.
        let foo = PkgId::root(sym("foo"));
        let baz = scoped("baz", &[sym("foo")]);
        let bar = scoped_vis("bar", &[sym("foo"), sym("baz")], Visibility::Internal);
        let mut map = ScopedUseMap::new();
        map.insert((foo, sym("baz")), baz);
        map.insert((baz, sym("bar")), bar);
        let err = resolve_path_use(&map, foo, &[sym("baz"), sym("bar")]).unwrap_err();
        match err {
            UseResolveError::VisibilityCeiling {
                target,
                target_scope,
                consumer,
            } => {
                assert_eq!(consumer, "foo");
                assert_eq!(target, "foo:baz:bar");
                assert_eq!(target_scope, "foo:baz");
            }
            other => panic!("expected VisibilityCeiling, got {other:?}"),
        }
    }

    #[test]
    fn internal_reach_in_allowed_from_within_subtree() {
        // baz (scope `foo:baz` — bar's own parent) may import its own internal bar.
        let baz = scoped("baz", &[sym("foo")]);
        let bar = scoped_vis("bar", &[sym("foo"), sym("baz")], Visibility::Internal);
        let mut map = ScopedUseMap::new();
        map.insert((baz, sym("bar")), bar);
        // baz's home scope is foo:baz; bar's owner_scope is foo:baz => prefix holds.
        let got = resolve_path_use(&map, baz, &[sym("bar")]).unwrap();
        assert_eq!(got, bar);
    }

    #[test]
    fn use_resolves_locally_no_global_arbitration() {
        // foo (root) -> baz ; baz -> bar.  baz's `use bar` resolves to foo:baz:bar.
        let foo = PkgId::root(sym("foo"));
        let baz = scoped("baz", &[sym("foo")]);
        let bar_under_baz = scoped("bar", &[sym("foo"), sym("baz")]);
        let mut map = ScopedUseMap::new();
        map.insert((baz, sym("bar")), bar_under_baz);
        let got = resolve_use(&map, baz, sym("bar")).unwrap();
        assert_eq!(got, bar_under_baz);
        assert_eq!(got.fully_qualified(), "foo:baz:bar");
        // foo never declared `use bar`: no global fallback to baz's bar.
        assert!(resolve_use(&map, foo, sym("bar")).is_err());
    }

    #[test]
    fn same_name_distinct_per_consumer() {
        // Both foo and baz `use bar`, but they bind DIFFERENT scoped identities.
        let foo = PkgId::root(sym("foo"));
        let baz = scoped("baz", &[sym("foo")]);
        let bar_under_foo = scoped("bar", &[sym("foo")]);
        let bar_under_baz = scoped("bar", &[sym("foo"), sym("baz")]);
        let mut map = ScopedUseMap::new();
        map.insert((foo, sym("bar")), bar_under_foo);
        map.insert((baz, sym("bar")), bar_under_baz);
        let from_foo = resolve_use(&map, foo, sym("bar")).unwrap();
        let from_baz = resolve_use(&map, baz, sym("bar")).unwrap();
        assert_ne!(from_foo, from_baz);
        assert_eq!(from_foo.fully_qualified(), "foo:bar");
        assert_eq!(from_baz.fully_qualified(), "foo:baz:bar");
    }

    #[test]
    fn unresolved_use_names_the_consumer() {
        let foo = PkgId::root(sym("foo"));
        let map = ScopedUseMap::new();
        let err = resolve_use(&map, foo, sym("missing")).unwrap_err();
        match err {
            UseResolveError::Unresolved { consumer, name } => {
                assert_eq!(consumer, "foo");
                assert_eq!(name, sym("missing"));
            }
            other => panic!("expected Unresolved, got {other:?}"),
        }
    }

    #[test]
    fn build_item_pkgs_empty_deps_is_empty() {
        let names = [sym("main"), sym("helper_doit")];
        let out = build_item_pkgs(&HashMap::new(), names.iter());
        assert!(out.is_empty());
    }

    #[test]
    fn build_item_pkgs_attributes_by_module_prefix() {
        let dep = scoped("helper", &[]);
        let mut deps = HashMap::new();
        deps.insert(sym("helper"), dep);
        let names = [sym("main"), sym("helper_doit"), sym("helper_aux")];
        let out = build_item_pkgs(&deps, names.iter());
        // Dependency items are attributed; root items are omitted (fall to root).
        assert_eq!(out.get(&sym("helper_doit")), Some(&dep));
        assert_eq!(out.get(&sym("helper_aux")), Some(&dep));
        assert_eq!(out.get(&sym("main")), None);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn build_item_pkgs_longest_prefix_wins() {
        // Modules `a` and `a_b` both prefix `a_b_thing`; the longer must win so
        // `a_b`'s items never get mis-attributed to `a`.
        let a = scoped("a", &[]);
        let ab = scoped("a_b", &[]);
        assert_ne!(a, ab);
        let mut deps = HashMap::new();
        deps.insert(sym("a"), a);
        deps.insert(sym("a_b"), ab);
        let names = [sym("a_thing"), sym("a_b_thing")];
        let out = build_item_pkgs(&deps, names.iter());
        assert_eq!(out.get(&sym("a_thing")), Some(&a));
        assert_eq!(out.get(&sym("a_b_thing")), Some(&ab));
    }

    #[test]
    fn semantic_hash_empty_source_stable() {
        let scope = ScopePath::root();
        let version = ver(0);
        let h1 = compute_semantic_hash(sym("root"), scope, &version, b"", &[]);
        let h2 = compute_semantic_hash(sym("root"), scope, &version, b"", &[]);
        assert_eq!(h1, h2);
        assert_ne!(h1, [0u8; 32]);
    }
}
