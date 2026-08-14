use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Capability {
    FsRead(Option<String>),
    FsWrite(Option<String>),
    NetClient,
    NetServer,
    ProcessSpawn,
    EnvRead,
    Clock,
    Time,
    Random,
    FfiUnsafe,
    State(Option<String>),
    IndirectCall,
}

impl Capability {
    pub fn class(&self) -> &'static str {
        match self {
            Capability::FsRead(_) => "fs.read",
            Capability::FsWrite(_) => "fs.write",
            Capability::NetClient => "net.client",
            Capability::NetServer => "net.server",
            Capability::ProcessSpawn => "process.spawn",
            Capability::EnvRead => "env.read",
            Capability::Clock => "clock",
            Capability::Time => "time",
            Capability::Random => "random",
            Capability::FfiUnsafe => "ffi.unsafe",
            Capability::State(_) => "state",
            Capability::IndirectCall => "indirect-call",
        }
    }

    pub fn render(&self) -> String {
        match self {
            Capability::FsRead(Some(p)) => format!("fs.read '{p}'"),
            Capability::FsWrite(Some(p)) => format!("fs.write '{p}'"),
            Capability::State(Some(r)) => format!("state <{r}>"),
            Capability::IndirectCall => {
                "a call through a function value (the callee's capabilities cannot \
                 be classified yet; call a named function instead)"
                    .to_string()
            }
            other => other.class().to_string(),
        }
    }

    pub fn subsumed_by(&self, other: &Capability) -> bool {
        match (self, other) {
            (Capability::FsRead(a), Capability::FsRead(b)) => path_within(a, b),
            (Capability::FsWrite(a), Capability::FsWrite(b)) => path_within(a, b),
            (Capability::State(a), Capability::State(b)) => match b {
                None => true,
                Some(_) => a == b,
            },
            (x, y) => x == y,
        }
    }
}

fn normalize_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if matches!(out.last(), Some(&"..")) || out.is_empty() {
                    out.push("..");
                } else {
                    out.pop();
                }
            }
            s => out.push(s),
        }
    }
    out.join("/")
}

fn path_within(inner: &Option<String>, outer: &Option<String>) -> bool {
    match outer {
        None => true,
        Some(o) => match inner {
            None => false,
            Some(i) => {
                let no = normalize_path(o);
                let ni = normalize_path(i);
                ni == no || ni.starts_with(&format!("{no}/"))
            }
        },
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapSet {
    caps: BTreeSet<Capability>,
}

impl CapSet {
    pub fn new() -> Self {
        CapSet {
            caps: BTreeSet::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }

    pub fn len(&self) -> usize {
        self.caps.len()
    }

    pub fn insert(&mut self, c: Capability) {
        self.caps.insert(c);
    }

    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.caps.iter()
    }

    pub fn join(&mut self, other: &CapSet) {
        for c in &other.caps {
            self.caps.insert(c.clone());
        }
    }

    pub fn satisfies(&self, cap: &Capability) -> bool {
        self.caps.iter().any(|d| cap.subsumed_by(d))
    }

    pub fn contained_in(&self, ceiling: &CapSet) -> Vec<Capability> {
        self.caps
            .iter()
            .filter(|c| !ceiling.satisfies(c))
            .cloned()
            .collect()
    }

    pub fn render(&self) -> String {
        if self.caps.is_empty() {
            return "()".to_string();
        }
        self.caps
            .iter()
            .map(|c| c.render())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl FromIterator<Capability> for CapSet {
    fn from_iter<I: IntoIterator<Item = Capability>>(iter: I) -> Self {
        CapSet {
            caps: iter.into_iter().collect(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapRow {
    pub caps: CapSet,
    pub ffi_tainted: bool,
}

impl CapRow {
    pub fn empty() -> Self {
        CapRow::default()
    }

    pub fn join(&mut self, other: &CapRow) {
        self.caps.join(&other.caps);
        self.ffi_tainted |= other.ffi_tainted;
    }

    pub fn is_promotable(&self) -> bool {
        self.caps.is_empty() && !self.ffi_tainted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_containment_order() {
        let narrow = Capability::FsRead(Some("./assets/img/x.png".into()));
        let mid = Capability::FsRead(Some("./assets/img".into()));
        let wide = Capability::FsRead(Some("./assets".into()));
        let any = Capability::FsRead(None);

        assert!(narrow.subsumed_by(&mid));
        assert!(narrow.subsumed_by(&wide));
        assert!(narrow.subsumed_by(&any));
        assert!(mid.subsumed_by(&wide));
        assert!(!wide.subsumed_by(&mid));
        assert!(!wide.subsumed_by(&narrow));
    }

    #[test]
    fn normalization_strips_dot_segments() {
        let a = Capability::FsRead(Some("./assets/./x".into()));
        let b = Capability::FsRead(Some("assets".into()));
        assert!(a.subsumed_by(&b));
    }

    #[test]
    fn distinct_classes_never_subsume() {
        assert!(!Capability::NetClient.subsumed_by(&Capability::NetServer));
        assert!(Capability::NetClient.subsumed_by(&Capability::NetClient));
    }

    #[test]
    fn unscoped_state_is_top() {
        let region = Capability::State(Some("foo:bar".into()));
        let any = Capability::State(None);
        assert!(region.subsumed_by(&any));
        assert!(!any.subsumed_by(&region));
        let other = Capability::State(Some("foo:baz".into()));
        assert!(!region.subsumed_by(&other));
    }

    #[test]
    fn set_join_is_union() {
        let mut a: CapSet = [Capability::NetClient].into_iter().collect();
        let b: CapSet = [Capability::FsRead(None)].into_iter().collect();
        a.join(&b);
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn containment_reports_uncovered() {
        let derived: CapSet = [
            Capability::NetClient,
            Capability::FsWrite(Some("./out/log".into())),
        ]
        .into_iter()
        .collect();
        let declared: CapSet = [Capability::FsWrite(Some("./out".into()))]
            .into_iter()
            .collect();
        let bad = derived.contained_in(&declared);
        assert_eq!(bad.len(), 1);
        assert_eq!(bad[0], Capability::NetClient);
    }

    #[test]
    fn scoped_write_satisfies_wider_declaration() {
        let derived: CapSet = [Capability::FsWrite(Some("./out/a/b".into()))]
            .into_iter()
            .collect();
        let declared: CapSet = [Capability::FsWrite(Some("./out".into()))]
            .into_iter()
            .collect();
        assert!(derived.contained_in(&declared).is_empty());
    }

    #[test]
    fn empty_row_is_promotable() {
        assert!(CapRow::empty().is_promotable());
        let mut tainted = CapRow::empty();
        tainted.ffi_tainted = true;
        assert!(!tainted.is_promotable());
        let nonempty = CapRow {
            caps: [Capability::Clock].into_iter().collect(),
            ffi_tainted: false,
        };
        assert!(!nonempty.is_promotable());
    }

    #[test]
    fn render_empty_is_unit() {
        assert_eq!(CapSet::new().render(), "()");
    }
}
