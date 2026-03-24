use crate::pkg::SemVer;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct LockEntry {
    pub name: String,
    pub url: String,
    pub version: SemVer,
    pub commit: String,
    pub deps: Vec<LockEntry>,
}

#[derive(Debug, Clone)]
pub struct Lockfile {
    pub entries: Vec<LockEntry>,
}

impl Lockfile {
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        let mut i = 0;
        let lines: Vec<&str> = input.lines().collect();
        while i < lines.len() {
            let line = lines[i];
            if line.is_empty() || line.starts_with('#') {
                i += 1;
                continue;
            }
            if !line.starts_with(' ') {
                let entry = Self::parse_entry(line, 0)?;
                let mut top = entry;
                i += 1;
                while i < lines.len() && lines[i].starts_with("  ") {
                    let child_line = lines[i].trim();
                    if !child_line.is_empty() && !child_line.starts_with('#') {
                        top.deps.push(Self::parse_entry(child_line, 2)?);
                    }
                    i += 1;
                }
                entries.push(top);
            } else {
                i += 1;
            }
        }
        Ok(Lockfile { entries })
    }

    fn parse_entry(line: &str, _indent: usize) -> Result<LockEntry, String> {
        let parts: Vec<&str> = line.trim().split_whitespace().collect();
        if parts.len() < 4 {
            return Err(format!("jade.lock: invalid entry: {line}"));
        }
        Ok(LockEntry {
            name: parts[0].to_string(),
            url: parts[1].to_string(),
            version: SemVer::parse(parts[2])?,
            commit: parts[3].to_string(),
            deps: Vec::new(),
        })
    }

    pub fn write(&self) -> String {
        let mut out = String::from("# jade.lock — auto-generated, do not edit\n");
        let mut sorted = self.entries.clone();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in &sorted {
            out.push_str(&format!(
                "{} {} {} {}\n",
                entry.name, entry.url, entry.version, entry.commit
            ));
            let mut deps = entry.deps.clone();
            deps.sort_by(|a, b| a.name.cmp(&b.name));
            for dep in &deps {
                out.push_str(&format!(
                    "  {} {} {} {}\n",
                    dep.name, dep.url, dep.version, dep.commit
                ));
            }
        }
        out
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        let input = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse(&input)
    }

    pub fn find(&self, name: &str) -> Option<&LockEntry> {
        self.entries.iter().find(|e| e.name == name)
    }
}
