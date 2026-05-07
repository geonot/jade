//! On-disk caching of compiled artifacts (HIR, MIR, object files) for incremental builds.

use crate::intern::Symbol;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;

fn ensure_allowed_dep_url(url: &str) -> Result<(), String> {
    if url.starts_with("https://") {
        return Ok(());
    }
    let allow_non_https = std::env::var("JADE_ALLOW_NON_HTTPS_DEPS")
        .ok()
        .map(|v| v == "1")
        .unwrap_or(false);
    if allow_non_https
        && (url.starts_with("http://")
            || url.starts_with("ssh://")
            || url.starts_with("git@")
            || url.starts_with("file://"))
    {
        return Ok(());
    }
    Err(format!(
        "dependency URL '{url}' is not allowed; use https:// or set JADE_ALLOW_NON_HTTPS_DEPS=1"
    ))
}

use crate::lock::{LockEntry, Lockfile};
use crate::pkg::{Dependency, Package};

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new() -> Self {
        let root = dirs_cache().join("jade").join("cache");
        Cache { root }
    }

    pub fn package_path(&self, dep: &Dependency) -> PathBuf {
        self.make_path(&dep.url, &dep.version)
    }

    pub fn package_path_from_entry(&self, entry: &LockEntry) -> PathBuf {
        self.make_path(&entry.url, &entry.version)
    }

    fn make_path(&self, url: &str, version: &crate::pkg::SemVer) -> PathBuf {
        self.root.join(url_to_dir(url)).join(version.to_string())
    }

    pub fn is_cached(&self, dep: &Dependency) -> bool {
        self.package_path(dep).exists()
    }

    pub fn fetch(&self, dep: &Dependency) -> Result<String, String> {
        ensure_allowed_dep_url(&dep.url)?;
        let dir = self.package_path(dep);
        if dir.exists() {
            return Self::read_commit(&dir);
        }
        let tag = format!("v{}", dep.version);
        self.fetch_tag(dep, &tag)
    }

    fn fetch_tag(&self, dep: &Dependency, tag: &str) -> Result<String, String> {
        let dir = self.package_path(dep);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("cannot clear cache dir {}: {e}", dir.display()))?;
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create cache dir {}: {e}", dir.display()))?;

        let output = Command::new("git")
            .args(["clone", "--depth=1", "--branch", tag, &dep.url])
            .arg(&dir)
            .output()
            .map_err(|e| format!("git clone failed: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("git clone {} failed: {stderr}", dep.url));
        }
        Self::read_commit(&dir)
    }

    fn fetch_pinned_commit(&self, dep: &Dependency, commit: &str) -> Result<String, String> {
        ensure_allowed_dep_url(&dep.url)?;
        let dir = self.package_path(dep);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| format!("cannot clear cache dir {}: {e}", dir.display()))?;
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create cache dir {}: {e}", dir.display()))?;

        let clone = Command::new("git")
            .args(["clone", "--no-checkout", &dep.url])
            .arg(&dir)
            .output()
            .map_err(|e| format!("git clone failed: {e}"))?;
        if !clone.status.success() {
            let stderr = String::from_utf8_lossy(&clone.stderr);
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("git clone {} failed: {stderr}", dep.url));
        }

        let checkout = Command::new("git")
            .args(["checkout", "--detach", commit])
            .current_dir(&dir)
            .output()
            .map_err(|e| format!("git checkout failed: {e}"))?;
        if !checkout.status.success() {
            let stderr = String::from_utf8_lossy(&checkout.stderr);
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!(
                "git checkout {} at commit {} failed: {stderr}",
                dep.url, commit
            ));
        }

        let actual = Self::read_commit(&dir)?;
        if actual != commit {
            return Err(format!(
                "pinned commit mismatch for {}: expected {}, got {}",
                dep.name, commit, actual
            ));
        }
        Ok(actual)
    }

    fn read_commit(dir: &std::path::Path) -> Result<String, String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .map_err(|e| format!("git rev-parse failed: {e}"))?;
        if !output.status.success() {
            return Err("git rev-parse HEAD failed".into());
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn resolve(
        &self,
        pkg: &Package,
        existing_lock: Option<&Lockfile>,
    ) -> Result<Lockfile, String> {
        let mut entries = Vec::new();
        let mut resolving = HashSet::new();
        for dep in &pkg.requires {
            let entry = self.resolve_dep(dep, existing_lock, &mut resolving)?;
            entries.push(entry);
        }
        Ok(Lockfile { entries })
    }

    fn resolve_dep(
        &self,
        dep: &Dependency,
        existing_lock: Option<&Lockfile>,
        resolving: &mut HashSet<Symbol>,
    ) -> Result<LockEntry, String> {
        if !resolving.insert(Symbol::intern(&dep.name)) {
            return Err(format!("circular dependency: {}", dep.name));
        }
        let commit = if let Some(lock) = existing_lock {
            if let Some(entry) = lock.find(&dep.name) {
                if entry.version == dep.version {
                    let dir = self.package_path(dep);
                    if !self.is_cached(dep) {
                        self.fetch_pinned_commit(dep, &entry.commit)?;
                    } else {
                        let actual = Self::read_commit(&dir)?;
                        if actual != entry.commit {
                            self.fetch_pinned_commit(dep, &entry.commit)?;
                        }
                    }
                    resolving.remove(&Symbol::intern(&dep.name));
                    return Ok(entry.clone());
                }
            }
            self.fetch(dep)?
        } else {
            self.fetch(dep)?
        };

        let pkg_dir = self.package_path(dep);
        let sub_proj_file = pkg_dir.join("project.jade");
        let mut sub_deps = Vec::new();
        if sub_proj_file.exists() {
            let sub_pkg = Package::from_project_file(&sub_proj_file)?;
            for sub_dep in &sub_pkg.requires {
                let sub_entry = self.resolve_dep(sub_dep, existing_lock, resolving)?;
                sub_deps.push(sub_entry);
            }
        }

        resolving.remove(&Symbol::intern(&dep.name));
        Ok(LockEntry {
            name: dep.name.clone(),
            url: dep.url.clone(),
            version: dep.version.clone(),
            commit,
            deps: sub_deps,
        })
    }
}

pub fn build_package_map(cache: &Cache, lockfile: &Lockfile) -> HashMap<Symbol, PathBuf> {
    let mut map = HashMap::new();
    for entry in &lockfile.entries {
        collect_paths(cache, entry, &mut map);
    }
    map
}

fn collect_paths(cache: &Cache, entry: &LockEntry, map: &mut HashMap<Symbol, PathBuf>) {
    map.insert(Symbol::intern(&entry.name), cache.package_path_from_entry(entry));
    for dep in &entry.deps {
        collect_paths(cache, dep, map);
    }
}

fn url_to_dir(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .replace(':', "_")
}

fn dirs_cache() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache")
    } else {
        PathBuf::from("/tmp")
    }
}
