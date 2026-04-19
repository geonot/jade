use lasso::{Rodeo, Spur};
use std::cell::RefCell;

thread_local! {
    static INTERNER: RefCell<Rodeo> = RefCell::new(Rodeo::default());
}

/// A cheaply-copyable interned string handle.
/// Resolves to `&str` via the thread-local interner.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(Spur);

impl Symbol {
    /// Intern a string, returning a Symbol handle.
    pub fn intern(s: &str) -> Self {
        INTERNER.with(|r| Symbol(r.borrow_mut().get_or_intern(s)))
    }

    /// Resolve a Symbol back to its string contents.
    pub fn as_str(self) -> String {
        INTERNER.with(|r| r.borrow().resolve(&self.0).to_string())
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Symbol({})", self.as_str())
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_str())
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
