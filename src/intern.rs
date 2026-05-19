use lasso::{Rodeo, Spur};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};

thread_local! {
    static INTERNER: RefCell<Rodeo> = RefCell::new(Rodeo::default());
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Symbol(Spur);

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        INTERNER.with(|r| {
            r.borrow().resolve(&self.0).hash(state);
        });
    }
}

impl Symbol {
    pub fn intern(s: &str) -> Self {
        INTERNER.with(|r| Symbol(r.borrow_mut().get_or_intern(s)))
    }

    pub fn as_str(self) -> String {
        INTERNER.with(|r| r.borrow().resolve(&self.0).to_string())
    }

    pub fn with_str<R>(self, f: impl FnOnce(&str) -> R) -> R {
        let s = self.as_str();
        f(&s)
    }

    pub fn starts_with(self, prefix: &str) -> bool {
        self.with_str(|s| s.starts_with(prefix))
    }

    pub fn ends_with(self, suffix: &str) -> bool {
        self.with_str(|s| s.ends_with(suffix))
    }

    pub fn contains_str(self, pat: &str) -> bool {
        self.with_str(|s| s.contains(pat))
    }

    pub fn len(self) -> usize {
        self.with_str(|s| s.len())
    }

    pub fn is_empty(self) -> bool {
        self.with_str(|s| s.is_empty())
    }

    pub fn join_vec(syms: &[Symbol], sep: &str) -> String {
        syms.iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(sep)
    }

    pub fn strip_prefix(self, prefix: &str) -> Option<Symbol> {
        self.with_str(|s| s.strip_prefix(prefix).map(Symbol::intern))
    }
}

impl Default for Symbol {
    fn default() -> Self {
        Symbol::intern("")
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Symbol({})", self.as_str())
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        INTERNER.with(|r| f.write_str(r.borrow().resolve(&self.0)))
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.with_str(|s| s == *other)
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.with_str(|s| s == other)
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Symbol::intern(s)
    }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self {
        Symbol::intern(&s)
    }
}

impl PartialEq<String> for Symbol {
    fn eq(&self, other: &String) -> bool {
        self.with_str(|s| s == other.as_str())
    }
}

impl equivalent::Equivalent<Symbol> for str {
    fn equivalent(&self, key: &Symbol) -> bool {
        key.with_str(|s| s == self)
    }
}

impl equivalent::Equivalent<Symbol> for String {
    fn equivalent(&self, key: &Symbol) -> bool {
        key.with_str(|s| s == self.as_str())
    }
}
