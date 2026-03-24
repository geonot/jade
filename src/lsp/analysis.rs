use std::collections::HashMap;

use crate::ast::{self, Span};
use crate::lexer::Lexer;
use crate::parser::Parser;

/// Lightweight analysis result for a single file.
pub struct FileAnalysis {
    pub symbols: Vec<Symbol>,
    pub diagnostics: Vec<LspDiag>,
    /// Map from definition name → (type signature, span)
    pub defs: HashMap<String, (String, Span)>,
}

pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub span: Span,
    pub detail: String,
    pub children: Vec<Symbol>,
}

pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Field,
    Constant,
}

pub struct LspDiag {
    pub line: u32,
    pub col: u32,
    pub end_col: u32,
    pub message: String,
    pub severity: DiagSeverity,
}

pub enum DiagSeverity {
    Error,
    Warning,
}

/// Analyze a Jade source file: lex → parse, extract symbols and diagnostics.
/// Does NOT do codegen (fast enough for interactive use).
pub fn analyze(src: &str) -> FileAnalysis {
    let mut diagnostics = Vec::new();
    let mut symbols = Vec::new();
    let mut defs = HashMap::new();

    // Lex
    let tokens = match Lexer::new(src).tokenize() {
        Ok(t) => t,
        Err(e) => {
            diagnostics.push(LspDiag {
                line: 1,
                col: 1,
                end_col: 1,
                message: e.to_string(),
                severity: DiagSeverity::Error,
            });
            return FileAnalysis { symbols, diagnostics, defs };
        }
    };

    // Parse
    let prog = match Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(e) => {
            diagnostics.push(LspDiag {
                line: 1,
                col: 1,
                end_col: 1,
                message: e.to_string(),
                severity: DiagSeverity::Error,
            });
            return FileAnalysis { symbols, diagnostics, defs };
        }
    };

    // Extract symbols from declarations
    for d in &prog.decls {
        match d {
            ast::Decl::Fn(f) => {
                let sig = fn_signature(f);
                defs.insert(f.name.clone(), (sig.clone(), f.span));
                symbols.push(Symbol {
                    name: f.name.clone(),
                    kind: SymbolKind::Function,
                    span: f.span,
                    detail: sig,
                    children: Vec::new(),
                });
            }
            ast::Decl::Type(td) => {
                let mut children = Vec::new();
                for field in &td.fields {
                    let ty_str = field
                        .ty
                        .as_ref()
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "unknown".into());
                    children.push(Symbol {
                        name: field.name.clone(),
                        kind: SymbolKind::Field,
                        span: field.span,
                        detail: ty_str,
                        children: Vec::new(),
                    });
                }
                for m in &td.methods {
                    let sig = fn_signature(m);
                    let method_name = format!("{}_{}", td.name, m.name);
                    defs.insert(method_name, (sig.clone(), m.span));
                    children.push(Symbol {
                        name: m.name.clone(),
                        kind: SymbolKind::Function,
                        span: m.span,
                        detail: sig,
                        children: Vec::new(),
                    });
                }
                defs.insert(td.name.clone(), (format!("type {}", td.name), td.span));
                symbols.push(Symbol {
                    name: td.name.clone(),
                    kind: SymbolKind::Struct,
                    span: td.span,
                    detail: format!("type {} ({} fields)", td.name, td.fields.len()),
                    children,
                });
            }
            ast::Decl::Enum(ed) => {
                let mut children = Vec::new();
                for v in &ed.variants {
                    children.push(Symbol {
                        name: v.name.clone(),
                        kind: SymbolKind::Field,
                        span: v.span,
                        detail: String::new(),
                        children: Vec::new(),
                    });
                }
                defs.insert(ed.name.clone(), (format!("enum {}", ed.name), ed.span));
                symbols.push(Symbol {
                    name: ed.name.clone(),
                    kind: SymbolKind::Enum,
                    span: ed.span,
                    detail: format!("enum {} ({} variants)", ed.name, ed.variants.len()),
                    children,
                });
            }
            ast::Decl::Const(name, _expr, span) => {
                defs.insert(name.clone(), (format!("const {name}"), *span));
                symbols.push(Symbol {
                    name: name.clone(),
                    kind: SymbolKind::Constant,
                    span: *span,
                    detail: "constant".into(),
                    children: Vec::new(),
                });
            }
            ast::Decl::Extern(ef) => {
                let sig = extern_signature(ef);
                defs.insert(ef.name.clone(), (sig.clone(), ef.span));
                symbols.push(Symbol {
                    name: ef.name.clone(),
                    kind: SymbolKind::Function,
                    span: ef.span,
                    detail: sig,
                    children: Vec::new(),
                });
            }
            _ => {}
        }
    }

    FileAnalysis { symbols, diagnostics, defs }
}

/// Find the definition name at a given (1-based) line and column.
pub fn find_ident_at(src: &str, line: u32, col: u32) -> Option<String> {
    let target_line = src.lines().nth((line.saturating_sub(1)) as usize)?;
    let col0 = (col.saturating_sub(1)) as usize;
    if col0 >= target_line.len() {
        return None;
    }
    let bytes = target_line.as_bytes();
    if !is_ident_byte(bytes[col0]) {
        return None;
    }
    let mut start = col0;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = col0;
    while end < bytes.len() && is_ident_byte(bytes[end]) {
        end += 1;
    }
    Some(target_line[start..end].to_string())
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn fn_signature(f: &ast::Fn) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| match &p.ty {
            Some(t) => format!("{}: {t}", p.name),
            None => p.name.clone(),
        })
        .collect();
    let ret = f
        .ret
        .as_ref()
        .map(|t| format!(" -> {t}"))
        .unwrap_or_default();
    format!("*{}({}){}", f.name, params.join(", "), ret)
}

fn extern_signature(ef: &ast::ExternFn) -> String {
    let params: Vec<String> = ef
        .params
        .iter()
        .map(|(name, ty)| format!("{name}: {ty}"))
        .collect();
    let ret = format!(" -> {}", ef.ret);
    format!("extern *{}({}){}", ef.name, params.join(", "), ret)
}

/// Build a keyword/builtin completion list for Jade.
pub fn completions_for_context() -> Vec<(String, &'static str)> {
    vec![
        ("if".into(), "keyword"),
        ("else".into(), "keyword"),
        ("elif".into(), "keyword"),
        ("while".into(), "keyword"),
        ("for".into(), "keyword"),
        ("in".into(), "keyword"),
        ("loop".into(), "keyword"),
        ("break".into(), "keyword"),
        ("continue".into(), "keyword"),
        ("return".into(), "keyword"),
        ("is".into(), "keyword"),
        ("isnt".into(), "keyword"),
        ("equals".into(), "keyword"),
        ("match".into(), "keyword"),
        ("type".into(), "keyword"),
        ("enum".into(), "keyword"),
        ("trait".into(), "keyword"),
        ("impl".into(), "keyword"),
        ("use".into(), "keyword"),
        ("extern".into(), "keyword"),
        ("true".into(), "keyword"),
        ("false".into(), "keyword"),
        ("none".into(), "keyword"),
        ("and".into(), "keyword"),
        ("or".into(), "keyword"),
        ("not".into(), "keyword"),
        ("as".into(), "keyword"),
        ("dispatch".into(), "keyword"),
        ("select".into(), "keyword"),
        ("spawn".into(), "keyword"),
        ("send".into(), "keyword"),
        ("log".into(), "function"),
        ("vec".into(), "function"),
        ("channel".into(), "function"),
        ("to_string".into(), "function"),
        ("char".into(), "function"),
    ]
}
