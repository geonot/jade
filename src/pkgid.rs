use crate::intern::Symbol;
use crate::pkg::SemVer;
use std::cell::RefCell;
use std::collections::HashMap;

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
}
