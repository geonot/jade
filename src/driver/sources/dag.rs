#![allow(dead_code)]
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::intern::Symbol;
use crate::pkg::{Dependency, SemVer};
use crate::pkgid::{
    PackageRecord, PkgId, ScopePath, ScopedUseMap, Visibility, compute_semantic_hash,
};

pub fn resolve_scoped_pkg_ids(dag: &ResolutionDag) -> ScopedUseMap {
    let mut map = ScopedUseMap::new();
    for node in &dag.nodes {
        for &dep_id in &node.deps {
            let dep_rec = dep_id.record();
            map.insert((node.pkg_id, dep_rec.name), dep_id);
        }
    }
    map
}

#[derive(Debug, Clone)]
pub struct ResolvedNode {
    pub pkg_id: PkgId,
    pub path: PathBuf,
    pub deps: Vec<PkgId>,
}

#[derive(Debug, Default)]
pub struct ResolutionDag {
    pub nodes: Vec<ResolvedNode>,
    pub by_id: HashMap<PkgId, usize>,
}

impl ResolutionDag {
    pub fn get(&self, id: PkgId) -> Option<&ResolvedNode> {
        self.by_id.get(&id).map(|&i| &self.nodes[i])
    }
}

pub fn flatten_workspace(
    root: PkgId,
    root_path: &std::path::Path,
    root_deps: &[Dependency],
    pkg_paths: &HashMap<Symbol, PathBuf>,
) -> Result<ResolutionDag, String> {
    let mut dag = ResolutionDag::default();
    let mut visiting: HashSet<(Symbol, String)> = HashSet::new();
    let rec = root.record();
    resolve_node(
        rec.name,
        rec.owner_scope,
        &rec.version,
        root_path,
        root_deps,
        rec.visibility,
        pkg_paths,
        &mut dag,
        &mut visiting,
        Some(root),
    )?;
    reject_multi_version(&dag)?;
    Ok(dag)
}

fn reject_multi_version(dag: &ResolutionDag) -> Result<(), String> {
    let mut majors: HashMap<Symbol, HashSet<u32>> = HashMap::new();
    for node in &dag.nodes {
        let rec = node.pkg_id.record();
        majors
            .entry(rec.name)
            .or_default()
            .insert(rec.version.major);
    }
    for node in &dag.nodes {
        let rec = node.pkg_id.record();
        if let Some(set) = majors.get(&rec.name)
            && set.len() > 1
        {
            let mut found: Vec<String> = dag
                .nodes
                .iter()
                .map(|n| n.pkg_id.record())
                .filter(|r| r.name == rec.name)
                .map(|r| r.version.to_string())
                .collect();
            found.sort();
            found.dedup();
            return Err(format!(
                "multi-version coexistence of '{}' ({}) is not yet supported; it is \
                     gated on coherence/traits (lamp.md §5.7.6). Pick one major.",
                rec.name,
                found.join(" and ")
            ));
        }
    }
    Ok(())
}

pub fn package_sources_digest(path: &std::path::Path) -> Result<[u8; 32], String> {
    crate::source_manifest::SourceManifest::scan(path, crate::source_manifest::PACKAGE_FILE_LIMIT)
        .map(|m| m.digest())
        .map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn resolve_node(
    name: Symbol,
    owner_scope: ScopePath,
    version: &SemVer,
    path: &std::path::Path,
    deps: &[Dependency],
    visibility: Visibility,
    pkg_paths: &HashMap<Symbol, PathBuf>,
    dag: &mut ResolutionDag,
    visiting: &mut HashSet<(Symbol, String)>,
    fixed: Option<PkgId>,
) -> Result<PkgId, String> {
    let cycle_key = (name, version.to_string());
    if visiting.contains(&cycle_key) {
        return Err(format!(
            "dependency cycle detected at package '{name}@{version}'"
        ));
    }
    visiting.insert(cycle_key.clone());

    let child_scope = if owner_scope.is_root() {
        ScopePath::intern(&[name])
    } else {
        let mut segs = owner_scope.segments();
        segs.push(name);
        ScopePath::intern(&segs)
    };

    let mut dep_ids: Vec<PkgId> = Vec::new();
    for dep in deps {
        let dep_name = Symbol::intern(&dep.name);
        let dep_path = match pkg_paths.get(&dep_name) {
            Some(p) => p.clone(),
            None => {
                return Err(format!(
                    "package '{}' requires '{}' but it was not found in the resolved set",
                    name, dep.name
                ));
            }
        };
        let child_deps = load_child_deps(&dep_path);
        let child_vis = load_visibility(&dep_path);
        let dep_id = resolve_node(
            dep_name,
            child_scope,
            &dep.version,
            &dep_path,
            &child_deps,
            child_vis,
            pkg_paths,
            dag,
            visiting,
            None,
        )?;
        dep_ids.push(dep_id);
    }

    let pkg_id = match fixed {
        Some(id) => id,
        None => {
            let src = package_sources_digest(path)?;
            let mut sorted_dep_hashes: Vec<[u8; 32]> =
                dep_ids.iter().map(|d| d.record().semantic_hash).collect();
            sorted_dep_hashes.sort();
            let hash = compute_semantic_hash(name, owner_scope, version, &src, &sorted_dep_hashes);
            PkgId::intern(PackageRecord {
                name,
                owner_scope,
                version: version.clone(),
                semantic_hash: hash,
                visibility,
            })
        }
    };

    if let Some(&existing_idx) = dag.by_id.get(&pkg_id) {
        let existing = &dag.nodes[existing_idx];
        if existing.path != path {
            return Err(format!(
                "package identity collision: '{}@{}' under scope '{}' resolves to two \
                 different source paths:\n  1: {}\n  2: {}",
                name,
                version,
                owner_scope,
                existing.path.display(),
                path.display()
            ));
        }
        visiting.remove(&cycle_key);
        return Ok(pkg_id);
    }

    let idx = dag.nodes.len();
    dag.nodes.push(ResolvedNode {
        pkg_id,
        path: path.to_path_buf(),
        deps: dep_ids,
    });
    dag.by_id.insert(pkg_id, idx);
    visiting.remove(&cycle_key);
    Ok(pkg_id)
}

fn load_child_deps(pkg_path: &std::path::Path) -> Vec<Dependency> {
    let project_jn = pkg_path.join("project.jn");
    if !project_jn.exists() {
        return Vec::new();
    }
    match crate::driver::project::ProjectConfig::from_file(&project_jn) {
        Ok(cfg) => cfg.requires,
        Err(_) => Vec::new(),
    }
}

fn load_visibility(pkg_path: &std::path::Path) -> Visibility {
    let project_jn = pkg_path.join("project.jn");
    if !project_jn.exists() {
        return Visibility::Public;
    }
    match crate::driver::project::ProjectConfig::from_file(&project_jn) {
        Ok(cfg) => cfg.visibility,
        Err(_) => Visibility::Public,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

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

    fn flatten_workspace(
        name: Symbol,
        path: &std::path::Path,
        deps: &[Dependency],
        pkg_paths: &HashMap<Symbol, PathBuf>,
    ) -> Result<ResolutionDag, String> {
        let digest = package_sources_digest(path)?;
        let root = PkgId::intern(PackageRecord {
            name,
            owner_scope: ScopePath::root(),
            version: ver(0),
            semantic_hash: compute_semantic_hash(name, ScopePath::root(), &ver(0), &digest, &[]),
            visibility: load_visibility(path),
        });
        super::flatten_workspace(root, path, deps, pkg_paths)
    }

    fn write_src(dir: &std::path::Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn single_root_no_deps() {
        let tmp = TempDir::new().unwrap();
        write_src(tmp.path(), "main.jn", "*main\n  log 1\n");
        let dag = flatten_workspace(sym("root"), tmp.path(), &[], &HashMap::new()).unwrap();
        assert_eq!(dag.nodes.len(), 1);
        assert_eq!(dag.nodes[0].pkg_id.name(), sym("root"));
        assert_ne!(dag.nodes[0].pkg_id.record().semantic_hash, [0u8; 32]);
    }

    #[test]
    fn two_roots_distinct_ids() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        write_src(tmp1.path(), "a.jn", "fn foo\n  1\n");
        write_src(tmp2.path(), "b.jn", "fn bar\n  2\n");
        let v = ver(0);
        let scope = ScopePath::root();
        let name = sym("pkg");
        let d1 = package_sources_digest(tmp1.path()).unwrap();
        let d2 = package_sources_digest(tmp2.path()).unwrap();
        let h1 = compute_semantic_hash(name, scope, &v, &d1, &[]);
        let h2 = compute_semantic_hash(name, scope, &v, &d2, &[]);
        assert_ne!(
            h1, h2,
            "different source bytes must produce different hashes"
        );
    }

    #[test]
    fn dep_included_in_dag() {
        let root_dir = TempDir::new().unwrap();
        let dep_dir = TempDir::new().unwrap();
        write_src(root_dir.path(), "main.jn", "*main\n  log 1\n");
        write_src(dep_dir.path(), "lib.jn", "fn helper\n  42\n");

        let mut pkg_paths = HashMap::new();
        pkg_paths.insert(sym("helper"), dep_dir.path().to_path_buf());

        let dep = Dependency {
            name: "helper".into(),
            url: "https://example.com/helper".into(),
            version: ver(1),
        };
        let dag = flatten_workspace(sym("myapp"), root_dir.path(), &[dep], &pkg_paths).unwrap();
        assert_eq!(dag.nodes.len(), 2);
    }

    #[test]
    fn scoped_use_map_resolves_dep_locally() {
        let root_dir = TempDir::new().unwrap();
        let dep_dir = TempDir::new().unwrap();
        write_src(root_dir.path(), "main.jn", "*main\n  log 1\n");
        write_src(dep_dir.path(), "lib.jn", "fn helper\n  42\n");

        let mut pkg_paths = HashMap::new();
        pkg_paths.insert(sym("helper"), dep_dir.path().to_path_buf());

        let dep = Dependency {
            name: "helper".into(),
            url: "https://example.com/helper".into(),
            version: ver(1),
        };
        let dag = flatten_workspace(sym("myapp"), root_dir.path(), &[dep], &pkg_paths).unwrap();
        let map = resolve_scoped_pkg_ids(&dag);

        let consumer = dag
            .nodes
            .iter()
            .find(|n| n.pkg_id.name() == sym("myapp"))
            .unwrap()
            .pkg_id;
        let resolved = crate::pkgid::resolve_use(&map, consumer, sym("helper")).unwrap();
        assert_eq!(resolved.name(), sym("helper"));
        assert_eq!(resolved.fully_qualified(), "myapp:helper");

        assert!(crate::pkgid::resolve_use(&map, consumer, sym("nope")).is_err());
    }

    fn write_manifest(dir: &std::path::Path, body: &str) {
        std::fs::write(dir.join("project.jn"), body).unwrap();
    }

    fn build_three_tier(bar_visibility: &str) -> (ResolutionDag, PkgId) {
        let root = TempDir::new().unwrap();
        let baz = TempDir::new().unwrap();
        let bar = TempDir::new().unwrap();
        let root = Box::leak(Box::new(root));
        let baz = Box::leak(Box::new(baz));
        let bar = Box::leak(Box::new(bar));

        write_src(root.path(), "main.jn", "*main\n  log 1\n");
        write_manifest(
            root.path(),
            "name is 'foo'\nversion is '1.0.0'\nrequire('baz', 'x', '1.0.0')\n",
        );
        write_src(baz.path(), "lib.jn", "fn b\n  1\n");
        write_manifest(
            baz.path(),
            "name is 'baz'\nversion is '1.0.0'\nrequire('bar', 'x', '1.0.0')\n",
        );
        write_src(bar.path(), "lib.jn", "fn r\n  2\n");
        write_manifest(
            bar.path(),
            &format!("name is 'bar'\nversion is '1.0.0'\nvisibility is '{bar_visibility}'\n"),
        );

        let mut pkg_paths = HashMap::new();
        pkg_paths.insert(sym("baz"), baz.path().to_path_buf());
        pkg_paths.insert(sym("bar"), bar.path().to_path_buf());

        let root_deps = vec![Dependency {
            name: "baz".into(),
            url: "x".into(),
            version: ver(1),
        }];
        let dag = flatten_workspace(sym("foo"), root.path(), &root_deps, &pkg_paths).unwrap();
        let foo = dag
            .nodes
            .iter()
            .find(|n| n.pkg_id.name() == sym("foo"))
            .unwrap()
            .pkg_id;
        (dag, foo)
    }

    #[test]
    fn e2e_public_reach_in_resolves() {
        let (dag, foo) = build_three_tier("public");
        let map = resolve_scoped_pkg_ids(&dag);
        let got = crate::pkgid::resolve_path_use(&map, foo, &[sym("baz"), sym("bar")]).unwrap();
        assert_eq!(got.fully_qualified(), "foo:baz:bar");
    }

    #[test]
    fn e2e_internal_reach_in_is_hard_error() {
        let (dag, foo) = build_three_tier("internal");
        let map = resolve_scoped_pkg_ids(&dag);
        let err = crate::pkgid::resolve_path_use(&map, foo, &[sym("baz"), sym("bar")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("visibility internal"), "{msg}");
        assert!(msg.contains("foo:baz:bar"), "{msg}");
    }

    #[test]
    fn e2e_internal_reach_in_allowed_from_parent() {
        let (dag, _foo) = build_three_tier("internal");
        let map = resolve_scoped_pkg_ids(&dag);
        let baz = dag
            .nodes
            .iter()
            .find(|n| n.pkg_id.name() == sym("baz"))
            .unwrap()
            .pkg_id;

        let got = crate::pkgid::resolve_path_use(&map, baz, &[sym("bar")]).unwrap();
        assert_eq!(got.fully_qualified(), "foo:baz:bar");
    }

    #[test]
    fn two_live_majors_hard_rejected() {
        let root = TempDir::new().unwrap();
        let mid = TempDir::new().unwrap();
        let foo = TempDir::new().unwrap();

        write_src(root.path(), "main.jn", "*main\n  log 1\n");
        write_manifest(
            root.path(),
            "name is 'root'\nversion is '1.0.0'\n\
             require('foo', 'x', '1.2.0')\nrequire('mid', 'x', '1.0.0')\n",
        );
        write_src(mid.path(), "lib.jn", "fn m\n  1\n");
        write_manifest(
            mid.path(),
            "name is 'mid'\nversion is '1.0.0'\nrequire('foo', 'x', '2.0.0')\n",
        );
        write_src(foo.path(), "lib.jn", "fn f\n  1\n");
        write_manifest(foo.path(), "name is 'foo'\nversion is '1.0.0'\n");

        let mut pkg_paths = HashMap::new();
        pkg_paths.insert(sym("mid"), mid.path().to_path_buf());
        pkg_paths.insert(sym("foo"), foo.path().to_path_buf());

        let root_deps = vec![
            Dependency {
                name: "foo".into(),
                url: "x".into(),
                version: SemVer {
                    major: 1,
                    minor: 2,
                    patch: 0,
                },
            },
            Dependency {
                name: "mid".into(),
                url: "x".into(),
                version: ver(1),
            },
        ];
        let err = flatten_workspace(sym("root"), root.path(), &root_deps, &pkg_paths).unwrap_err();
        assert!(err.contains("multi-version coexistence of 'foo'"), "{err}");
        assert!(err.contains("1.2.0"), "{err}");
        assert!(err.contains("2.0.0"), "{err}");
        assert!(err.contains("coherence/traits"), "{err}");
    }

    #[test]
    fn same_major_minor_diff_accepted() {
        let root = TempDir::new().unwrap();
        let foo = TempDir::new().unwrap();
        write_src(root.path(), "main.jn", "*main\n  log 1\n");
        write_manifest(
            root.path(),
            "name is 'root'\nversion is '1.0.0'\nrequire('foo', 'x', '1.2.0')\n",
        );
        write_src(foo.path(), "lib.jn", "fn f\n  1\n");
        write_manifest(foo.path(), "name is 'foo'\nversion is '1.0.0'\n");

        let mut pkg_paths = HashMap::new();
        pkg_paths.insert(sym("foo"), foo.path().to_path_buf());

        let root_deps = vec![Dependency {
            name: "foo".into(),
            url: "x".into(),
            version: SemVer {
                major: 1,
                minor: 2,
                patch: 0,
            },
        }];
        assert!(flatten_workspace(sym("root"), root.path(), &root_deps, &pkg_paths).is_ok());
    }

    #[test]
    fn missing_dep_is_error() {
        let tmp = TempDir::new().unwrap();
        write_src(tmp.path(), "main.jn", "*main\n  log 1\n");
        let dep = Dependency {
            name: "missing".into(),
            url: "https://example.com/missing".into(),
            version: ver(1),
        };
        let result = flatten_workspace(sym("myapp"), tmp.path(), &[dep], &HashMap::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }
}
