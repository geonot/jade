use super::*;
use crate::pkgid::{PkgId, ScopePath, PackageRecord, compute_semantic_hash};

pub(in crate::driver) fn load_packages(base_dir: &std::path::Path) -> HashMap<Symbol, PathBuf> {
    load_packages_with_ids(base_dir).0
}

pub(in crate::driver) fn load_packages_with_ids(
    base_dir: &std::path::Path,
) -> (HashMap<Symbol, PathBuf>, HashMap<Symbol, PkgId>) {
    let project_root = find_project_root(base_dir).unwrap_or_else(|| base_dir.to_path_buf());
    let project_jinn = project_root.join("project.jn");
    let requires = if project_jinn.exists() {
        match ProjectConfig::from_file(&project_jinn) {
            Ok(cfg) => cfg.requires,
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    if requires.is_empty() {
        return (HashMap::new(), HashMap::new());
    }
    let pkg = Package {
        name: String::new(),
        version: SemVer {
            major: 0,
            minor: 0,
            patch: 0,
        },
        author: None,
        requires,
    };
    let lock_file = project_root.join("jinn.lock");
    let existing_lock = if lock_file.exists() {
        Some(Lockfile::from_file(&lock_file).unwrap_or_else(|e| die(&format!("jinn.lock: {e}"))))
    } else {
        None
    };
    let cache = Cache::new();
    let resolved = cache
        .resolve(&pkg, existing_lock.as_ref())
        .unwrap_or_else(|e| die(&format!("resolve: {e}")));
    let lock_content = resolved.write();
    fs::write(&lock_file, &lock_content).unwrap_or_else(|e| die(&format!("write lock: {e}")));
    let path_map = build_package_map(&cache, &resolved);
    let id_map = build_pkg_id_map(&path_map);
    (path_map, id_map)
}

fn build_pkg_id_map(path_map: &HashMap<Symbol, PathBuf>) -> HashMap<Symbol, PkgId> {
    let mut ids = HashMap::new();
    for (&name, path) in path_map {
        let src = crate::driver::sources::dag::source_bytes_for(path);
        let hash = compute_semantic_hash(
            name,
            ScopePath::root(),
            &SemVer { major: 0, minor: 0, patch: 0 },
            &src,
            &[],
        );
        let visibility = {
            let project_jn = path.join("project.jn");
            if project_jn.exists() {
                crate::driver::project::ProjectConfig::from_file(&project_jn)
                    .map(|c| c.visibility)
                    .unwrap_or_default()
            } else {
                crate::pkgid::Visibility::Public
            }
        };
        let pkg_id = PkgId::intern(PackageRecord {
            name,
            owner_scope: ScopePath::root(),
            version: SemVer { major: 0, minor: 0, patch: 0 },
            semantic_hash: hash,
            visibility,
        });
        ids.insert(name, pkg_id);
    }
    ids
}
