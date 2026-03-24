use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;

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
        let url_path = url_to_dir(&dep.url);
        self.root.join(url_path).join(dep.version.to_string())
    }

    pub fn package_path_from_entry(&self, entry: &LockEntry) -> PathBuf {
        let url_path = url_to_dir(&entry.url);
        self.root.join(url_path).join(entry.version.to_string())
    }

    pub fn is_cached(&self, dep: &Dependency) -> bool {
        self.package_path(dep).exists()
    }

    pub fn fetch(&self, dep: &Dependency) -> Result<String, String> {
        let dir = self.package_path(dep);
        if dir.exists() {
            return Self::read_commit(&dir);
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create cache dir {}: {e}", dir.display()))?;
        let tag = format!("v{}", dep.version);
        let output = Command::new("git")
            .args(["clone", "--depth=1", "--branch", &tag, &dep.url])
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
        resolving: &mut HashSet<String>,
    ) -> Result<LockEntry, String> {
        if !resolving.insert(dep.name.clone()) {
            return Err(format!("circular dependency: {}", dep.name));
        }
        let commit = if let Some(lock) = existing_lock {
            if let Some(entry) = lock.find(&dep.name) {
                if entry.version.to_string() == dep.version.to_string() {
                    if !self.is_cached(dep) {
                        self.fetch(dep)?;
                    }
                    resolving.remove(&dep.name);
                    return Ok(entry.clone());
                }
            }
            self.fetch(dep)?
        } else {
            self.fetch(dep)?
        };

        let pkg_dir = self.package_path(dep);
        let sub_pkg_file = pkg_dir.join("jade.pkg");
        let mut sub_deps = Vec::new();
        if sub_pkg_file.exists() {
            let sub_pkg = Package::from_file(&sub_pkg_file)?;
            for sub_dep in &sub_pkg.requires {
                let sub_entry = self.resolve_dep(sub_dep, existing_lock, resolving)?;
                sub_deps.push(sub_entry);
            }
        }

        resolving.remove(&dep.name);
        Ok(LockEntry {
            name: dep.name.clone(),
            url: dep.url.clone(),
            version: dep.version.clone(),
            commit,
            deps: sub_deps,
        })
    }
}

pub fn build_package_map(cache: &Cache, lockfile: &Lockfile) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    for entry in &lockfile.entries {
        collect_paths(cache, entry, &mut map);
    }
    map
}

fn collect_paths(cache: &Cache, entry: &LockEntry, map: &mut HashMap<String, PathBuf>) {
    map.insert(entry.name.clone(), cache.package_path_from_entry(entry));
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
