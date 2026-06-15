#![allow(dead_code)]
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::intern::Symbol;
use crate::pkg::{Dependency, SemVer};
use crate::pkgid::{PackageRecord, PkgId, ScopePath, compute_semantic_hash};

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
        pkg_paths,
        &mut dag,
        &mut visiting,
    )?;
    Ok(dag)
}

fn source_bytes_for(path: &std::path::Path) -> Vec<u8> {
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
        let dep_id = resolve_node(
            dep_name,
            child_scope,
            &dep.version,
            &dep_path,
            &child_deps,
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
    fn missing_dep_is_error() {
        let tmp = TempDir::new().unwrap();
        write_src(tmp.path(), "main.jn", "*main\n  log 1\n");
        let dep = Dependency { name: "missing".into(), url: "https://example.com/missing".into(), version: ver(1) };
        let result = flatten_workspace(sym("myapp"), tmp.path(), &[dep], &HashMap::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }
}
