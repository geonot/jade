//! Incremental and parallel compilation.
//!
//! Hash each function's HIR and cache generated object code.
//! On recompile, skip unchanged functions by loading cached artifacts.
//! Parallel codegen partitions functions across threads.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::hir;
use crate::types::Type;

// ── Cache Key / HIR Hashing ──

/// Compute a stable 64-bit hash of a function's HIR plus dependency signatures.
pub fn function_cache_key(func: &hir::Fn, dep_sigs: &HashMap<String, u64>) -> u64 {
    let mut hasher = StableHasher::new();
    hash_function(func, &mut hasher);
    // Include dependency signatures so callee type changes invalidate callers.
    let mut deps: Vec<_> = dep_sigs.iter().collect();
    deps.sort_by_key(|(k, _)| *k);
    for (name, sig) in deps {
        name.hash(&mut hasher);
        sig.hash(&mut hasher);
    }
    hasher.finish()
}

/// Compute a signature hash for a function (name + param types + return type).
pub fn function_signature(func: &hir::Fn) -> u64 {
    let mut hasher = StableHasher::new();
    func.name.hash(&mut hasher);
    for p in &func.params {
        hash_type(&p.ty, &mut hasher);
    }
    hash_type(&func.ret, &mut hasher);
    hasher.finish()
}

fn hash_function(func: &hir::Fn, h: &mut StableHasher) {
    func.name.hash(h);
    func.params.len().hash(h);
    for p in &func.params {
        p.name.hash(h);
        hash_type(&p.ty, h);
    }
    hash_type(&func.ret, h);
    for stmt in &func.body {
        hash_stmt(stmt, h);
    }
}

fn hash_type(ty: &Type, h: &mut StableHasher) {
    // Debug repr as hash source — sufficient for cache invalidation.
    format!("{ty:?}").hash(h);
}

fn hash_stmt(stmt: &hir::Stmt, h: &mut StableHasher) {
    std::mem::discriminant(stmt).hash(h);
    format!("{:?}", stmt).hash(h);
}

/// FNV-1a hasher for stable, deterministic hashing.
struct StableHasher {
    state: u64,
}

impl StableHasher {
    fn new() -> Self {
        StableHasher {
            state: 0xcbf29ce484222325, // FNV offset basis
        }
    }
}

impl Hasher for StableHasher {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= b as u64;
            self.state = self.state.wrapping_mul(0x100000001b3); // FNV prime
        }
    }

    fn finish(&self) -> u64 {
        self.state
    }
}

// ── Artifact Cache ──

/// Manages cached compilation artifacts on disk.
pub struct ArtifactCache {
    cache_dir: PathBuf,
}

impl ArtifactCache {
    /// Create a new artifact cache at `~/.cache/jade/artifacts/`.
    pub fn new() -> Self {
        let cache_dir = cache_root().join("jade").join("artifacts");
        ArtifactCache { cache_dir }
    }

    /// Create a new cache at a custom directory (for testing).
    pub fn with_dir(dir: PathBuf) -> Self {
        ArtifactCache { cache_dir: dir }
    }

    /// Look up a cached bitcode file. Returns `Some(path)` if valid.
    pub fn lookup(&self, func_name: &str, key: u64) -> Option<PathBuf> {
        let path = self.artifact_path(func_name, key);
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    /// Store a bitcode artifact for the given function.
    pub fn store(&self, func_name: &str, key: u64, data: &[u8]) -> std::io::Result<()> {
        let path = self.artifact_path(func_name, key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, data)?;
        // Clean up old versions of this function
        self.gc_old(func_name, key);
        Ok(())
    }

    fn artifact_path(&self, func_name: &str, key: u64) -> PathBuf {
        // Use a subdirectory per function to keep things organized
        self.cache_dir
            .join(sanitize_name(func_name))
            .join(format!("{:016x}.bc", key))
    }

    /// Remove old cached versions (keep only current key).
    fn gc_old(&self, func_name: &str, current_key: u64) {
        let dir = self.cache_dir.join(sanitize_name(func_name));
        let current_name = format!("{:016x}.bc", current_key);
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy() != current_name {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    /// Remove all cached artifacts.
    pub fn clear(&self) -> std::io::Result<()> {
        if self.cache_dir.exists() {
            std::fs::remove_dir_all(&self.cache_dir)?;
        }
        Ok(())
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn cache_root() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache")
    } else {
        PathBuf::from("/tmp")
    }
}

// ── Incremental Build Logic ──

/// Determine which functions need recompilation.
/// Returns a list of function indices that have changed since the last build.
pub fn compute_dirty_set(
    program: &hir::Program,
    cache: &ArtifactCache,
) -> (Vec<usize>, HashMap<String, u64>) {
    // 1. Compute signatures for all functions
    let mut signatures: HashMap<String, u64> = HashMap::new();
    for f in &program.fns {
        signatures.insert(f.name.clone(), function_signature(f));
    }

    // 2. Compute cache keys and check which are dirty
    let mut dirty = Vec::new();
    let mut keys: HashMap<String, u64> = HashMap::new();
    for (i, f) in program.fns.iter().enumerate() {
        let key = function_cache_key(f, &signatures);
        keys.insert(f.name.clone(), key);
        if cache.lookup(&f.name, key).is_none() {
            dirty.push(i);
        }
    }

    (dirty, keys)
}

// ── Parallel Codegen ──

/// Partition function indices into N roughly-equal chunks for parallel codegen.
pub fn partition_work(total: usize, num_threads: usize) -> Vec<std::ops::Range<usize>> {
    if total == 0 || num_threads == 0 {
        return vec![];
    }
    let n = num_threads.min(total);
    let chunk = total / n;
    let remainder = total % n;
    let mut ranges = Vec::with_capacity(n);
    let mut start = 0;
    for i in 0..n {
        let extra = if i < remainder { 1 } else { 0 };
        let end = start + chunk + extra;
        ranges.push(start..end);
        start = end;
    }
    ranges
}

/// Get the number of threads for parallel codegen (CPU cores, capped at 16).
pub fn codegen_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().min(16))
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_work() {
        let ranges = partition_work(10, 3);
        assert_eq!(ranges.len(), 3);
        let total: usize = ranges.iter().map(|r| r.len()).sum();
        assert_eq!(total, 10);
    }

    #[test]
    fn test_partition_work_more_threads_than_items() {
        let ranges = partition_work(3, 10);
        assert_eq!(ranges.len(), 3);
        let total: usize = ranges.iter().map(|r| r.len()).sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn test_partition_work_empty() {
        let ranges = partition_work(0, 4);
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_stable_hasher_deterministic() {
        let mut h1 = StableHasher::new();
        "hello".hash(&mut h1);
        let mut h2 = StableHasher::new();
        "hello".hash(&mut h2);
        assert_eq!(h1.finish(), h2.finish());
    }

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("foo::bar<i64>"), "foo__bar_i64_");
        assert_eq!(sanitize_name("simple"), "simple");
    }

    #[test]
    fn test_artifact_cache_roundtrip() {
        let dir = std::env::temp_dir().join("jade_test_cache");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = ArtifactCache::with_dir(dir.clone());

        assert!(cache.lookup("test_fn", 42).is_none());
        cache.store("test_fn", 42, b"fake bitcode").unwrap();
        assert!(cache.lookup("test_fn", 42).is_some());
        assert!(cache.lookup("test_fn", 99).is_none());

        cache.clear().unwrap();
        assert!(cache.lookup("test_fn", 42).is_none());
    }
}
