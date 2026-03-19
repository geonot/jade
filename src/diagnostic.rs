use crate::ast::Span;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: msg.into(),
            span: None,
            notes: vec![],
        }
    }
    pub fn warning(msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: msg.into(),
            span: None,
            notes: vec![],
        }
    }
    pub fn at(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
    pub fn note(mut self, n: impl Into<String>) -> Self {
        self.notes.push(n.into());
        self
    }

    pub fn render(&self, filename: &str, source: &str) -> String {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let mut out = String::new();
        out.push_str(&format!("{sev}: {}\n", self.message));
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
        for n in &self.notes {
            out.push_str(&format!(" = note: {n}\n"));
        }
        out
    }
}
