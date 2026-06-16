use crate::intern::Symbol;
use crate::pkg::SemVer;
use std::cell::RefCell;
use std::collections::HashMap;
use blake3::Hasher;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScopePath(u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PkgId(u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRecord {
    pub name: Symbol,
    pub owner_scope: ScopePath,
    pub version: SemVer,
    pub semantic_hash: [u8; 32],
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
        let h: String = r.semantic_hash[..4].iter().map(|b| format!("{b:02x}")).collect();
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
}

impl std::fmt::Display for UseResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UseResolveError::Unresolved { consumer, name } => write!(
                f,
                "unresolved import 'use {name}' in package '{consumer}': add '{name}' to its \
                 project.jn requires"
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
    map.get(&(consumer, name)).copied().ok_or_else(|| UseResolveError::Unresolved {
        consumer: consumer.fully_qualified(),
        name,
    })
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
        });
        let v2 = PkgId::intern(PackageRecord {
            name: sym("foo"),
            owner_scope: scope,
            version: ver(2),
            semantic_hash: [0u8; 32],
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
            semantic_hash: [0xab, 0xcd, 0xef, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
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
        })
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
        }
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
