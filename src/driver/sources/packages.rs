//! Package manifest loading for driver source resolution.

use super::*;

pub(in crate::driver) fn load_packages(base_dir: &std::path::Path) -> HashMap<Symbol, PathBuf> {
    let project_root = find_project_root(base_dir).unwrap_or_else(|| base_dir.to_path_buf());
    let project_jade = project_root.join("project.jade");
    let requires = if project_jade.exists() {
        match ProjectConfig::from_file(&project_jade) {
            Ok(cfg) => cfg.requires,
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    if requires.is_empty() {
        return HashMap::new();
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
    let lock_file = project_root.join("jade.lock");
    let existing_lock = if lock_file.exists() {
        Some(Lockfile::from_file(&lock_file).unwrap_or_else(|e| die(&format!("jade.lock: {e}"))))
    } else {
        None
    };
    let cache = Cache::new();
    let resolved = cache
        .resolve(&pkg, existing_lock.as_ref())
        .unwrap_or_else(|e| die(&format!("resolve: {e}")));
    let lock_content = resolved.write();
    fs::write(&lock_file, &lock_content).unwrap_or_else(|e| die(&format!("write lock: {e}")));
    build_package_map(&cache, &resolved)
}
