use blake3::Hasher;
use std::path::{Path, PathBuf};

pub type Digest = [u8; 32];

pub const PACKAGE_FILE_LIMIT: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub len: u64,
    pub hash: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    Io { path: PathBuf, msg: String },
    Symlink(PathBuf),
    BadPath(PathBuf),
    TooMany { root: PathBuf, limit: usize },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io { path, msg } => write!(f, "cannot read {}: {msg}", path.display()),
            ManifestError::Symlink(p) => {
                write!(
                    f,
                    "symbolic link {} is not permitted in package sources",
                    p.display()
                )
            }
            ManifestError::BadPath(p) => write!(
                f,
                "source path {} is not valid UTF-8 or contains a control character",
                p.display()
            ),
            ManifestError::TooMany { root, limit } => write!(
                f,
                "{} contains more than {limit} source files",
                root.display()
            ),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceManifest {
    entries: Vec<FileEntry>,
}

impl SourceManifest {
    pub fn scan(root: &Path, limit: usize) -> Result<Self, ManifestError> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let rd = std::fs::read_dir(&dir).map_err(|e| io(&dir, e))?;
            for ent in rd {
                let ent = ent.map_err(|e| io(&dir, e))?;
                let path = ent.path();
                let name = ent.file_name();
                if name.to_string_lossy().starts_with('.') || name == "target" {
                    continue;
                }
                let ty = ent.file_type().map_err(|e| io(&path, e))?;
                if ty.is_symlink() {
                    return Err(ManifestError::Symlink(path));
                }
                if ty.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "jn") {
                    if files.len() == limit {
                        return Err(ManifestError::TooMany {
                            root: root.to_path_buf(),
                            limit,
                        });
                    }
                    files.push(path);
                }
            }
        }
        Self::of_files(root, &files)
    }

    pub fn of_files(base: &Path, files: &[PathBuf]) -> Result<Self, ManifestError> {
        let mut entries = Vec::with_capacity(files.len());
        for f in files {
            let rel = f.strip_prefix(base).unwrap_or(f);
            let path = render(rel).ok_or_else(|| ManifestError::BadPath(f.clone()))?;
            let bytes = std::fs::read(f).map_err(|e| io(f, e))?;
            entries.push(FileEntry {
                path,
                len: bytes.len() as u64,
                hash: *blake3::hash(&bytes).as_bytes(),
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries.dedup_by(|a, b| a.path == b.path);
        Ok(SourceManifest { entries })
    }

    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    pub fn digest(&self) -> Digest {
        let mut h = Hasher::new_derive_key("jinn:source-manifest:v1");
        h.update(&(self.entries.len() as u64).to_le_bytes());
        for e in &self.entries {
            h.update(&(e.path.len() as u64).to_le_bytes());
            h.update(e.path.as_bytes());
            h.update(&e.len.to_le_bytes());
            h.update(&e.hash);
        }
        *h.finalize().as_bytes()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("jinn-sources v1\n");
        for e in &self.entries {
            out.push_str(&format!("{}\t{}\t{}\n", hex(&e.hash), e.len, e.path));
        }
        out
    }

    pub fn from_text(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        if lines.next()? != "jinn-sources v1" {
            return None;
        }
        let mut entries = Vec::new();
        for line in lines {
            let mut parts = line.splitn(3, '\t');
            let hash = unhex(parts.next()?)?;
            let len = parts.next()?.parse().ok()?;
            let path = parts.next()?.to_string();
            entries.push(FileEntry { path, len, hash });
        }
        Some(SourceManifest { entries })
    }

    pub fn is_current(&self) -> bool {
        !self.entries.is_empty()
            && self.entries.iter().all(|e| {
                std::fs::read(&e.path).is_ok_and(|b| {
                    b.len() as u64 == e.len && *blake3::hash(&b).as_bytes() == e.hash
                })
            })
    }
}

fn io(path: &Path, e: std::io::Error) -> ManifestError {
    ManifestError::Io {
        path: path.to_path_buf(),
        msg: e.to_string(),
    }
}

fn render(rel: &Path) -> Option<String> {
    let mut out = String::new();
    for c in rel.components() {
        match c {
            std::path::Component::RootDir => out.push('/'),
            std::path::Component::Normal(s) => {
                let s = s.to_str()?;
                if s.chars().any(char::is_control) {
                    return None;
                }
                if !out.is_empty() && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(s);
            }
            _ => return None,
        }
    }
    (!out.is_empty() && out != "/").then_some(out)
}

pub fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Digest> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn digest(root: &Path) -> Digest {
        SourceManifest::scan(root, PACKAGE_FILE_LIMIT)
            .unwrap()
            .digest()
    }

    #[test]
    fn nested_edits_change_digest() {
        let t = TempDir::new().unwrap();
        write(t.path(), "main.jn", "a");
        write(t.path(), "sub/deep/m.jn", "b");
        let before = digest(t.path());
        write(t.path(), "sub/deep/m.jn", "c");
        assert_ne!(before, digest(t.path()));
    }

    #[test]
    fn partition_and_rename_are_distinguished() {
        let a = TempDir::new().unwrap();
        write(a.path(), "a.jn", "xy");
        write(a.path(), "b.jn", "z");
        let b = TempDir::new().unwrap();
        write(b.path(), "a.jn", "x");
        write(b.path(), "b.jn", "yz");
        let c = TempDir::new().unwrap();
        write(c.path(), "a.jn", "xy");
        write(c.path(), "c.jn", "z");
        let d = TempDir::new().unwrap();
        write(d.path(), "a.jn", "xyz");
        let all = [
            digest(a.path()),
            digest(b.path()),
            digest(c.path()),
            digest(d.path()),
        ];
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                assert_ne!(all[i], all[j]);
            }
        }
    }

    #[test]
    fn digest_is_location_independent_and_ignores_hidden_and_target() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        for t in [&a, &b] {
            write(t.path(), "main.jn", "m");
            write(t.path(), "lib/x.jn", "x");
            write(t.path(), "notes.txt", "n");
        }
        write(b.path(), ".git/x.jn", "ignored");
        write(b.path(), "target/x.jn", "ignored");
        assert_eq!(digest(a.path()), digest(b.path()));
        let m = SourceManifest::scan(a.path(), PACKAGE_FILE_LIMIT).unwrap();
        let paths: Vec<_> = m.entries().iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["lib/x.jn", "main.jn"]);
    }

    #[test]
    fn symlinks_unreadable_dirs_and_oversize_fail_closed() {
        let t = TempDir::new().unwrap();
        write(t.path(), "a.jn", "a");
        write(t.path(), "b.jn", "b");
        assert!(matches!(
            SourceManifest::scan(t.path(), 1),
            Err(ManifestError::TooMany { limit: 1, .. })
        ));
        std::os::unix::fs::symlink(t.path().join("a.jn"), t.path().join("l.jn")).unwrap();
        assert!(matches!(
            SourceManifest::scan(t.path(), PACKAGE_FILE_LIMIT),
            Err(ManifestError::Symlink(_))
        ));
        assert!(matches!(
            SourceManifest::scan(&t.path().join("missing"), PACKAGE_FILE_LIMIT),
            Err(ManifestError::Io { .. })
        ));
    }

    #[test]
    fn text_round_trip_detects_stale_inputs() {
        let t = TempDir::new().unwrap();
        write(t.path(), "a.jn", "one");
        let f = t.path().join("a.jn");
        let m = SourceManifest::of_files(Path::new(""), std::slice::from_ref(&f)).unwrap();
        let back = SourceManifest::from_text(&m.to_text()).unwrap();
        assert_eq!(back, m);
        assert!(back.is_current());
        write(t.path(), "a.jn", "two");
        assert!(!back.is_current());
        assert!(!SourceManifest::default().is_current());
        assert!(SourceManifest::from_text("").is_none());
        assert!(SourceManifest::from_text("jinn-sources v1\nzz\t1\tx\n").is_none());
    }

    #[test]
    fn duplicate_inputs_collapse() {
        let t = TempDir::new().unwrap();
        write(t.path(), "a.jn", "a");
        let f = t.path().join("a.jn");
        let one = SourceManifest::of_files(t.path(), std::slice::from_ref(&f)).unwrap();
        let two = SourceManifest::of_files(t.path(), &[f.clone(), f]).unwrap();
        assert_eq!(one.digest(), two.digest());
    }
}
