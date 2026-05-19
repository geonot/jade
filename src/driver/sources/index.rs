use super::*;

pub(in crate::driver) struct EntityIndex {
    pub(in crate::driver::sources) modules: HashMap<Symbol, PathBuf>,
}

impl EntityIndex {
    fn new() -> Self {
        Self {
            modules: HashMap::new(),
        }
    }

    fn scan_dir(&mut self, dir: &std::path::Path) {
        fn collect(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        collect(&path, files);
                    } else if path.extension().map_or(false, |e| e == "jn") {
                        files.push(path);
                    }
                }
            }
        }
        let mut files = Vec::new();
        collect(dir, &mut files);
        for file in files {
            self.scan_file(&file);
        }
    }

    fn scan_file(&mut self, path: &std::path::Path) {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            self.modules
                .entry(Symbol::intern(stem))
                .or_insert_with(|| path.to_path_buf());
        }
    }

    pub(in crate::driver) fn build(
        base_dir: &std::path::Path,
        packages: &HashMap<Symbol, PathBuf>,
    ) -> Self {
        let mut idx = Self::new();

        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                let std_dir = exe_dir.join("std");
                if std_dir.is_dir() {
                    idx.scan_dir(&std_dir);
                }

                if let Some(parent) = exe_dir.parent() {
                    let std_dir = parent.join("std");
                    if std_dir.is_dir() {
                        idx.scan_dir(&std_dir);
                    }
                    if let Some(grandparent) = parent.parent() {
                        let std_dir = grandparent.join("std");
                        if std_dir.is_dir() {
                            idx.scan_dir(&std_dir);
                        }
                    }
                }
            }
        }
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
            let std_dir = PathBuf::from(manifest).join("std");
            if std_dir.is_dir() {
                idx.scan_dir(&std_dir);
            }
        }
        let std_dir = base_dir.join("std");
        if std_dir.is_dir() {
            idx.scan_dir(&std_dir);
        }

        let source_dir = base_dir.join("source");
        if source_dir.is_dir() {
            idx.scan_dir(&source_dir);
        } else if base_dir
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s == "source")
            .unwrap_or(false)
        {
            idx.scan_dir(base_dir);
        }

        for (_, pkg_path) in packages {
            let source = pkg_path.join("source");
            if source.is_dir() {
                idx.scan_dir(&source);
            }
            let src = pkg_path.join("src");
            if src.is_dir() {
                idx.scan_dir(&src);
            }
        }

        if let Ok(pkg_paths) = std::env::var("JINN_PACKAGE_PATH") {
            for pkg_dir in pkg_paths.split(':') {
                let pkg_dir = PathBuf::from(pkg_dir);
                if pkg_dir.is_dir() {
                    idx.scan_dir(&pkg_dir);
                }
            }
        }

        idx
    }
}
