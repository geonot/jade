use crate::ast::Span;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// Structured error codes for the Jade compiler.
///
/// | Range     | Category              |
/// |-----------|-----------------------|
/// | E001–E099 | Syntax errors         |
/// | E100–E199 | Name resolution       |
/// | E200–E299 | Type errors           |
/// | E300–E399 | Ownership & borrow    |
/// | E400–E499 | Safety & FFI          |
/// | E500–E599 | Pattern matching      |
/// | E600–E699 | Memory management     |
/// | E700–E799 | Overflow & arithmetic |
/// | W001–W099 | General warnings      |
/// | W100–W199 | Performance warnings  |
/// | W200–W299 | Safety warnings       |
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErrorCode {
    // Syntax
    E001, // unexpected token
    E002, // invalid indentation
    E003, // unterminated string
    // Name resolution
    E100, // undefined variable
    E101, // undefined function
    E102, // undefined type
    E103, // duplicate definition
    // Type errors
    E200, // type mismatch
    E201, // cannot infer type
    E202, // invalid cast
    E203, // wrong number of arguments
    // Ownership & borrow
    E300, // use after move
    E301, // double mutable borrow
    E302, // move of borrowed value
    E303, // return of borrowed value
    E304, // invalid rc deref
    // Safety & FFI
    E400, // volatile on non-pointer type
    E401, // invalid extern signature
    E402, // raw pointer arithmetic
    // Pattern matching
    E500, // non-exhaustive match
    E501, // unreachable pattern
    E502, // duplicate pattern
    // Memory
    E600, // potential reference cycle
    E601, // weak upgrade may fail
    E602, // invalid layout annotation
    // Overflow
    E700, // integer overflow
    E701, // division by zero
    // Warnings
    W001, // unused variable
    W002, // unused import
    W100, // rc where owned would suffice
    W101, // allocation in hot loop
    W200, // potential cycle without weak
    W201, // volatile without fence
    W202, // unchecked overflow in user arithmetic
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// A secondary label pointing at a related source location.
#[derive(Debug, Clone)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<ErrorCode>,
    pub message: String,
    pub span: Option<Span>,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub suggestions: Vec<String>,
}

impl Diagnostic {
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code: None,
            message: msg.into(),
            span: None,
            labels: vec![],
            notes: vec![],
            suggestions: vec![],
        }
    }
    pub fn warning(msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code: None,
            message: msg.into(),
            span: None,
            labels: vec![],
            notes: vec![],
            suggestions: vec![],
        }
    }
    pub fn info(msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info,
            code: None,
            message: msg.into(),
            span: None,
            labels: vec![],
            notes: vec![],
            suggestions: vec![],
        }
    }
    pub fn with_code(mut self, code: ErrorCode) -> Self {
        self.code = Some(code);
        self
    }
    pub fn at(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
    pub fn label(mut self, span: Span, msg: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: msg.into(),
        });
        self
    }
    pub fn note(mut self, n: impl Into<String>) -> Self {
        self.notes.push(n.into());
        self
    }
    pub fn suggestion(mut self, s: impl Into<String>) -> Self {
        self.suggestions.push(s.into());
        self
    }

    pub fn render(&self, filename: &str, source: &str) -> String {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let mut out = String::new();
        match self.code {
            Some(code) => out.push_str(&format!("{sev}[{code}]: {}\n", self.message)),
            None => out.push_str(&format!("{sev}: {}\n", self.message)),
        }
        if let Some(sp) = self.span {
            out.push_str(&format!(" --> {filename}:{}:{}\n", sp.line, sp.col));
            if let Some(line_text) = source.lines().nth(sp.line.saturating_sub(1) as usize) {
                let line_num = sp.line;
                let pad = format!("{line_num}").len();
                out.push_str(&format!("{:>pad$} |\n", ""));
                out.push_str(&format!("{line_num} | {line_text}\n"));
                let col = sp.col.saturating_sub(1) as usize;
                let width = (sp.end.saturating_sub(sp.start)).max(1);
                out.push_str(&format!(
                    "{:>pad$} | {:>col$}{}\n",
                    "",
                    "",
                    "^".repeat(width)
                ));
            }
        }
        for lbl in &self.labels {
            if let Some(line_text) = source.lines().nth(lbl.span.line.saturating_sub(1) as usize) {
                let line_num = lbl.span.line;
                let pad = format!("{line_num}").len();
                let col = lbl.span.col.saturating_sub(1) as usize;
                let width = (lbl.span.end.saturating_sub(lbl.span.start)).max(1);
                out.push_str(&format!("{line_num} | {line_text}\n"));
                out.push_str(&format!(
                    "{:>pad$} | {:>col$}{} {}\n",
                    "",
                    "",
                    "-".repeat(width),
                    lbl.message
                ));
            }
        }
        for n in &self.notes {
            out.push_str(&format!(" = note: {n}\n"));
        }
        for s in &self.suggestions {
            out.push_str(&format!(" = help: {s}\n"));
        }
        out
    }
}
