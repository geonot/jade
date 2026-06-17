#![allow(dead_code)]
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::intern::Symbol;
use crate::pkg::{Dependency, SemVer};
use crate::pkgid::{PackageRecord, PkgId, ScopePath, ScopedUseMap, Visibility, compute_semantic_hash};

/// For each package A with a `use B` in its manifest, record the scoped PkgId
/// that `B` resolves to from A's perspective.
///
/// Key: `(consumer_pkg_id, dep_name_symbol)`
/// Value: the dep's `PkgId` as seen by the consumer (path-scoped under the
///        consumer's owner scope, per scope.md §2.1).

/// Build the `ScopedUseMap` for the given `ResolutionDag`.
///
/// For every node in the DAG, for every dep edge, the dep's identity is:
///   - `owner_scope` = child scope of the consumer's full path
///   - hash computed from the dep's source + its own transitive dep hashes
///
/// This is the structural realisation of scope.md §2.1 "local, deterministic,
/// no global arbitration".
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
    root_name: Symbol,
    root_path: &std::path::Path,
    root_deps: &[Dependency],
    pkg_paths: &HashMap<Symbol, PathBuf>,
) -> Result<ResolutionDag, String> {
    let mut dag = ResolutionDag::default();
    let mut visiting: HashSet<(Symbol, String)> = HashSet::new();
    resolve_node(
        root_name,
        ScopePath::root(),
        &SemVer { major: 0, minor: 0, patch: 0 },
        root_path,
        root_deps,
        load_visibility(root_path),
        pkg_paths,
        &mut dag,
        &mut visiting,
    )?;
    reject_multi_version(&dag)?;
    Ok(dag)
}

/// scope.md §5 / lamp.md §5.7.6: multi-version coexistence is deferred to
/// post-traits. The `PackageId` model already represents two live majors of the
/// same package distinctly (the version field forks the identity), so this is
/// the *only* thing holding the door shut — and lifting it later is purely
/// additive once the coherence/orphan rule lands.
///
/// Detect any package name that resolves to two **distinct major versions**
/// anywhere in the DAG and hard-reject with an honest diagnostic. Two instances
/// of the same major (only minor/patch differing) are not a coexistence hazard —
/// the resolver already unifies on the single highest compatible version per
/// major — so we key strictly on `name + major`.
fn reject_multi_version(dag: &ResolutionDag) -> Result<(), String> {
    let mut majors: HashMap<Symbol, HashSet<u32>> = HashMap::new();
    for node in &dag.nodes {
        let rec = node.pkg_id.record();
        majors.entry(rec.name).or_default().insert(rec.version.major);
    }
    for node in &dag.nodes {
        let rec = node.pkg_id.record();
        if let Some(set) = majors.get(&rec.name) {
            if set.len() > 1 {
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
    }
    Ok(())
}

pub fn source_bytes_for(path: &std::path::Path) -> Vec<u8> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "jn").unwrap_or(false))
            .collect();
        paths.sort();
        for p in &paths {
            if let Ok(bytes) = std::fs::read(p) {
                out.extend_from_slice(&bytes);
            }
        }
    }
    out
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
) -> Result<PkgId, String> {
    let cycle_key = (name, version.to_string());
    if visiting.contains(&cycle_key) {
        return Err(format!("dependency cycle detected at package '{name}@{version}'"));
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
        )?;
        dep_ids.push(dep_id);
    }

    let src = source_bytes_for(path);
    let mut sorted_dep_hashes: Vec<[u8; 32]> =
        dep_ids.iter().map(|d| d.record().semantic_hash).collect();
    sorted_dep_hashes.sort();

    let hash = compute_semantic_hash(name, owner_scope, version, &src, &sorted_dep_hashes);

    let pkg_id = PkgId::intern(PackageRecord {
        name,
        owner_scope,
        version: version.clone(),
        semantic_hash: hash,
        visibility,
    });

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
        SemVer { major: maj, minor: 0, patch: 0 }
    }

    fn write_src(dir: &std::path::Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn single_root_no_deps() {
        let tmp = TempDir::new().unwrap();
        write_src(tmp.path(), "main.jn", "*main\n  log 1\n");
        let dag = flatten_workspace(
            sym("root"),
            tmp.path(),
            &[],
            &HashMap::new(),
        )
        .unwrap();
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
        let h1 = compute_semantic_hash(name, scope, &v, &source_bytes_for(tmp1.path()), &[]);
        let h2 = compute_semantic_hash(name, scope, &v, &source_bytes_for(tmp2.path()), &[]);
        assert_ne!(h1, h2, "different source bytes must produce different hashes");
    }

    #[test]
    fn dep_included_in_dag() {
        let root_dir = TempDir::new().unwrap();
        let dep_dir = TempDir::new().unwrap();
        write_src(root_dir.path(), "main.jn", "*main\n  log 1\n");
        write_src(dep_dir.path(), "lib.jn", "fn helper\n  42\n");

        let mut pkg_paths = HashMap::new();
        pkg_paths.insert(sym("helper"), dep_dir.path().to_path_buf());

        let dep = Dependency { name: "helper".into(), url: "https://example.com/helper".into(), version: ver(1) };
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
        let resolved =
            crate::pkgid::resolve_use(&map, consumer, sym("helper")).unwrap();
        assert_eq!(resolved.name(), sym("helper"));
        assert_eq!(resolved.fully_qualified(), "myapp:helper");
        // Local: a name the consumer never required does not resolve.
        assert!(crate::pkgid::resolve_use(&map, consumer, sym("nope")).is_err());
    }

    fn write_manifest(dir: &std::path::Path, body: &str) {
        std::fs::write(dir.join("project.jn"), body).unwrap();
    }

    // foo (root) -> baz -> bar. Build the scoped graph from real manifests and
    // exercise the path-import + visibility-ceiling resolver end to end.
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
        let got =
            crate::pkgid::resolve_path_use(&map, foo, &[sym("baz"), sym("bar")]).unwrap();
        assert_eq!(got.fully_qualified(), "foo:baz:bar");
    }

    #[test]
    fn e2e_internal_reach_in_is_hard_error() {
        let (dag, foo) = build_three_tier("internal");
        let map = resolve_scoped_pkg_ids(&dag);
        let err = crate::pkgid::resolve_path_use(&map, foo, &[sym("baz"), sym("bar")])
            .unwrap_err();
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
        // baz is bar's parent: a direct `use bar` from baz satisfies the ceiling.
        let got = crate::pkgid::resolve_path_use(&map, baz, &[sym("bar")]).unwrap();
        assert_eq!(got.fully_qualified(), "foo:baz:bar");
    }

    // scope.md §5: root requires foo@1 directly and a sibling dep requires
    // foo@2 transitively. Two live majors of the same package => hard reject.
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
            Dependency { name: "foo".into(), url: "x".into(), version: SemVer { major: 1, minor: 2, patch: 0 } },
            Dependency { name: "mid".into(), url: "x".into(), version: ver(1) },
        ];
        let err = flatten_workspace(sym("root"), root.path(), &root_deps, &pkg_paths)
            .unwrap_err();
        assert!(err.contains("multi-version coexistence of 'foo'"), "{err}");
        assert!(err.contains("1.2.0"), "{err}");
        assert!(err.contains("2.0.0"), "{err}");
        assert!(err.contains("coherence/traits"), "{err}");
    }

    // Same package at differing minor/patch under one major is NOT a hazard:
    // it is the ordinary single-version resolution and must be accepted.
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
            version: SemVer { major: 1, minor: 2, patch: 0 },
        }];
        assert!(flatten_workspace(sym("root"), root.path(), &root_deps, &pkg_paths).is_ok());
    }

    #[test]
    fn missing_dep_is_error() {
        let tmp = TempDir::new().unwrap();
        write_src(tmp.path(), "main.jn", "*main\n  log 1\n");
        let dep = Dependency { name: "missing".into(), url: "https://example.com/missing".into(), version: ver(1) };
        let result = flatten_workspace(sym("myapp"), tmp.path(), &[dep], &HashMap::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }
}
