//! String interning for identifiers, paths, and symbol names.

use lasso::{Rodeo, Spur};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};

thread_local! {
    static INTERNER: RefCell<Rodeo> = RefCell::new(Rodeo::default());
}

/// A cheaply-copyable interned string handle.
/// Resolves to `&str` via the thread-local interner.
///
/// Hashes by string content (not Spur ID) so that cross-type lookups
/// on `IndexMap<Symbol,...>` with `&str` keys work correctly.
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
    /// Intern a string, returning a Symbol handle.
    pub fn intern(s: &str) -> Self {
        INTERNER.with(|r| Symbol(r.borrow_mut().get_or_intern(s)))
    }

    /// Resolve a Symbol back to its string contents (allocates).
    pub fn as_str(self) -> String {
        INTERNER.with(|r| r.borrow().resolve(&self.0).to_string())
    }

    /// Access the interned string via a callback without allocating.
    pub fn with_str<R>(self, f: impl FnOnce(&str) -> R) -> R {
        let s = self.as_str();
        f(&s)
    }

    /// Check if the interned string starts with a prefix.
    pub fn starts_with(self, prefix: &str) -> bool {
        self.with_str(|s| s.starts_with(prefix))
    }

    /// Check if the interned string ends with a suffix.
    pub fn ends_with(self, suffix: &str) -> bool {
        self.with_str(|s| s.ends_with(suffix))
    }

    /// Check if the interned string contains a substring.
    pub fn contains_str(self, pat: &str) -> bool {
        self.with_str(|s| s.contains(pat))
    }

    /// Get the length of the interned string.
    pub fn len(self) -> usize {
        self.with_str(|s| s.len())
    }

    /// Check if the interned string is empty.
    pub fn is_empty(self) -> bool {
        self.with_str(|s| s.is_empty())
    }

    /// Join a slice of Symbols with a separator into a String.
    pub fn join_vec(syms: &[Symbol], sep: &str) -> String {
        syms.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(sep)
    }

    /// Strip a prefix from the symbol's string, returning a new Symbol if matched.
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

// Allow HashMap<Symbol,...>.get("key") and .get(&string_var)
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
