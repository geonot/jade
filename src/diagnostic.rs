use crate::ast::Span;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErrorCode {
    E001,
    E002,
    E003,
    E100,
    E101,
    E102,
    E103,
    E200,
    E201,
    E202,
    E203,
    E300,
    E301,
    E302,
    E303,
    E304,
    E400,
    E401,
    E402,
    E500,
    E501,
    E502,
    E600,
    E601,
    E602,
    E700,
    E701,
    W001,
    W002,
    W100,
    W101,
    W200,
    W201,
    W202,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

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
