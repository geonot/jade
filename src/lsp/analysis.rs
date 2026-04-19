use std::collections::HashMap;

use crate::ast::{self, Span};
use crate::lexer::{Lexer, Token};
use crate::parser::Parser;

pub struct FileAnalysis {
    pub symbols: Vec<Symbol>,
    pub diagnostics: Vec<LspDiag>,
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

pub fn analyze(src: &str) -> FileAnalysis {
    let mut diagnostics = Vec::new();
    let mut symbols = Vec::new();
    let mut defs = HashMap::new();

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
            return FileAnalysis {
                symbols,
                diagnostics,
                defs,
            };
        }
    };

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
            return FileAnalysis {
                symbols,
                diagnostics,
                defs,
            };
        }
    };

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

    FileAnalysis {
        symbols,
        diagnostics,
        defs,
    }
}

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
            Some(t) => format!("{} as {t}", p.name),
            None => p.name.clone(),
        })
        .collect();
    let ret = f
        .ret
        .as_ref()
        .map(|t| format!(" returns {t}"))
        .unwrap_or_default();
    format!("*{}({}){}", f.name, params.join(", "), ret)
}

fn extern_signature(ef: &ast::ExternFn) -> String {
    let params: Vec<String> = ef
        .params
        .iter()
        .map(|(name, ty)| format!("{name} as {ty}"))
        .collect();
    let ret = format!(" returns {}", ef.ret);
    format!("extern *{}({}){}", ef.name, params.join(", "), ret)
}

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
        ("neq".into(), "keyword"),
        ("equals".into(), "keyword"),
        ("eq".into(), "keyword"),
        ("lt".into(), "keyword"),
        ("gt".into(), "keyword"),
        ("lte".into(), "keyword"),
        ("gte".into(), "keyword"),
        ("nlt".into(), "keyword"),
        ("ngt".into(), "keyword"),
        ("ngte".into(), "keyword"),
        ("nlte".into(), "keyword"),
        ("unless".into(), "keyword"),
        ("until".into(), "keyword"),
        ("returns".into(), "keyword"),
        ("mod".into(), "keyword"),
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

// ── Reference finding ──────────────────────────────────────────

pub struct IdentRef {
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

pub fn find_references(src: &str, name: &str) -> Vec<IdentRef> {
    let tokens = match Lexer::new(src).tokenize() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let mut refs = Vec::new();
    for sp in &tokens {
        if let Token::Ident(id) = &sp.token {
            if id == name {
                refs.push(IdentRef {
                    line: sp.span.line,
                    col: sp.span.col,
                    len: id.len() as u32,
                });
            }
        }
    }
    refs
}

// ── Semantic tokens ────────────────────────────────────────────

pub struct SemanticToken {
    pub delta_line: u32,
    pub delta_start: u32,
    pub length: u32,
    pub token_type: u32,
    pub token_modifiers: u32,
}

// Token type indices matching the legend in protocol.rs
pub const ST_KEYWORD: u32 = 0;
pub const ST_FUNCTION: u32 = 1;
pub const ST_VARIABLE: u32 = 2;
pub const ST_STRING: u32 = 3;
pub const ST_NUMBER: u32 = 4;
pub const ST_OPERATOR: u32 = 5;
pub const ST_TYPE: u32 = 6;
pub const ST_COMMENT: u32 = 7;
pub const ST_ENUM_MEMBER: u32 = 8;

pub fn semantic_tokens(src: &str) -> Vec<SemanticToken> {
    let tokens = match Lexer::new(src).tokenize() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };

    // Parse to find which identifiers are function names vs types vs variables
    let analysis = analyze(src);
    let mut fn_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut type_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for sym in &analysis.symbols {
        match sym.kind {
            SymbolKind::Function => { fn_names.insert(sym.name.clone()); }
            SymbolKind::Struct | SymbolKind::Enum => { type_names.insert(sym.name.clone()); }
            _ => {}
        }
    }

    let mut result = Vec::new();
    let mut prev_line: u32 = 0;
    let mut prev_col: u32 = 0;

    for sp in &tokens {
        let span = &sp.span;
        let (token_type, length) = match &sp.token {
            // Keywords
            Token::If | Token::Elif | Token::Else | Token::While | Token::For
            | Token::In | Token::Loop | Token::Break | Token::Continue | Token::Return
            | Token::Match | Token::When | Token::Type | Token::Enum | Token::Trait
            | Token::Impl | Token::Use | Token::As | Token::From | Token::To
            | Token::By | Token::Extern | Token::Is | Token::Returns
            | Token::And | Token::Or | Token::Not | Token::Equals | Token::Neq
            | Token::Unless | Token::Until | Token::Of | Token::Store | Token::Migration
            | Token::Insert | Token::Delete | Token::Set | Token::Transaction
            | Token::View | Token::Actor | Token::Spawn | Token::Send | Token::Receive
            | Token::Dispatch | Token::Yield | Token::Channel | Token::Close
            | Token::Select | Token::Stop | Token::Default | Token::Sim
            | Token::Supervisor | Token::Atomic | Token::Strict | Token::Try
            | Token::Global | Token::Pub => {
                (ST_KEYWORD, (span.end - span.start) as u32)
            }
            Token::True | Token::False | Token::None => {
                (ST_KEYWORD, (span.end - span.start) as u32)
            }
            // Strings
            Token::Str(_) => {
                (ST_STRING, (span.end - span.start) as u32)
            }
            // Numbers
            Token::Int(_) | Token::Float(_) | Token::CharLit(_) => {
                (ST_NUMBER, (span.end - span.start) as u32)
            }
            // Identifiers — classify based on analysis
            Token::Ident(name) => {
                let ty = if fn_names.contains(name.as_str()) {
                    ST_FUNCTION
                } else if type_names.contains(name.as_str()) {
                    ST_TYPE
                } else {
                    ST_VARIABLE
                };
                (ty, name.len() as u32)
            }
            // Built-in call keywords
            Token::Log => (ST_FUNCTION, 3),
            // Operators
            Token::Plus | Token::Minus | Token::Star | Token::Slash | Token::Percent
            | Token::Pipe | Token::Caret | Token::Ampersand | Token::Shl | Token::Shr
            | Token::Lt | Token::Gt | Token::LtEq | Token::GtEq
            | Token::StarStar | Token::Bang => {
                (ST_OPERATOR, (span.end - span.start) as u32)
            }
            Token::At | Token::AtKw => {
                (ST_OPERATOR, (span.end - span.start) as u32)
            }
            // Skip whitespace/structural tokens
            _ => continue,
        };

        if span.line == 0 || length == 0 {
            continue;
        }

        let line0 = span.line.saturating_sub(1);
        let col0 = span.col.saturating_sub(1);

        let delta_line = line0.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            col0.saturating_sub(prev_col)
        } else {
            col0
        };

        result.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers: 0,
        });

        prev_line = line0;
        prev_col = col0;
    }

    result
}

// ── Signature help ─────────────────────────────────────────────

pub struct SignatureInfo {
    pub label: String,
    pub params: Vec<String>,
    pub active_param: u32,
}

pub fn signature_at(src: &str, line: u32, col: u32) -> Option<SignatureInfo> {
    let analysis = analyze(src);
    let target_line = src.lines().nth((line.saturating_sub(1)) as usize)?;
    let col0 = (col.saturating_sub(1)) as usize;

    // Walk backwards from cursor to find the function name before '('
    let bytes = target_line.as_bytes();
    let mut depth = 0i32;
    let mut comma_count = 0u32;
    let mut paren_pos = None;

    let search_end = col0.min(bytes.len());
    for i in (0..search_end).rev() {
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    paren_pos = Some(i);
                    break;
                }
                depth -= 1;
            }
            b',' if depth == 0 => comma_count += 1,
            _ => {}
        }
    }

    let paren = paren_pos?;
    // Find function name before the '('
    let mut end = paren;
    while end > 0 && bytes[end - 1] == b' ' {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && is_ident_byte(bytes[start - 1]) {
        start -= 1;
    }
    if start == end {
        return None;
    }
    let fn_name = &target_line[start..end];

    // Look up the function signature
    let (sig, _) = analysis.defs.get(fn_name)?;
    let params = extract_params_from_sig(sig);

    Some(SignatureInfo {
        label: sig.clone(),
        params: params.clone(),
        active_param: comma_count.min(params.len().saturating_sub(1) as u32),
    })
}

fn extract_params_from_sig(sig: &str) -> Vec<String> {
    let start = sig.find('(').map(|i| i + 1).unwrap_or(0);
    let end = sig.rfind(')').unwrap_or(sig.len());
    if start >= end {
        return Vec::new();
    }
    sig[start..end]
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_function() {
        let src = "*add(a as i64, b as i64) returns i64\n    a + b\n";
        let a = analyze(src);
        assert!(a.diagnostics.is_empty());
        assert_eq!(a.symbols.len(), 1);
        assert_eq!(a.symbols[0].name, "add");
        assert!(a.defs.contains_key("add"));
    }

    #[test]
    fn test_analyze_struct() {
        let src = "type Point\n    x as f64\n    y as f64\n";
        let a = analyze(src);
        assert!(a.diagnostics.is_empty());
        assert_eq!(a.symbols.len(), 1);
        assert_eq!(a.symbols[0].name, "Point");
        assert_eq!(a.symbols[0].children.len(), 2);
    }

    #[test]
    fn test_analyze_enum() {
        let src = "enum Color\n    Red\n    Green\n    Blue\n";
        let a = analyze(src);
        assert!(a.diagnostics.is_empty());
        assert!(a.defs.contains_key("Color"));
    }

    #[test]
    fn test_analyze_error() {
        let src = "invalid @@@ syntax";
        let a = analyze(src);
        assert!(!a.diagnostics.is_empty());
    }

    #[test]
    fn test_find_ident_at() {
        let src = "*main()\n    x is 42\n    log(x)\n";
        assert_eq!(find_ident_at(src, 1, 2), Some("main".into()));
        assert_eq!(find_ident_at(src, 2, 5), Some("x".into()));
        assert_eq!(find_ident_at(src, 3, 9), Some("x".into()));
    }

    #[test]
    fn test_find_references() {
        let src = "*main()\n    x is 42\n    log(x)\n    y is x + 1\n";
        let refs = find_references(src, "x");
        assert_eq!(refs.len(), 3); // binding + log(x) + y is x
    }

    #[test]
    fn test_find_references_no_match() {
        let src = "*main()\n    x is 42\n";
        let refs = find_references(src, "nonexistent");
        assert!(refs.is_empty());
    }

    #[test]
    fn test_semantic_tokens() {
        let src = "*main()\n    x is 42\n";
        let toks = semantic_tokens(src);
        assert!(!toks.is_empty());
        // Should contain at least: function name, keyword (is), number (42)
        let has_keyword = toks.iter().any(|t| t.token_type == ST_KEYWORD);
        let has_number = toks.iter().any(|t| t.token_type == ST_NUMBER);
        assert!(has_keyword);
        assert!(has_number);
    }

    #[test]
    fn test_semantic_tokens_classifies_function() {
        let src = "*add(a as i64) returns i64\n    a\n\n*main()\n    add(1)\n";
        let toks = semantic_tokens(src);
        let has_fn = toks.iter().any(|t| t.token_type == ST_FUNCTION);
        assert!(has_fn);
    }

    #[test]
    fn test_signature_at() {
        let src = "*add(a as i64, b as i64) returns i64\n    a + b\n\n*main()\n    add(1, 2)\n";
        // Cursor inside add(1, |2)
        let info = signature_at(src, 5, 12);
        assert!(info.is_some());
        let info = info.unwrap();
        assert!(info.label.contains("add"));
        assert_eq!(info.params.len(), 2);
        assert_eq!(info.active_param, 1); // after the comma
    }

    #[test]
    fn test_signature_at_first_param() {
        let src = "*add(a as i64, b as i64) returns i64\n    a + b\n\n*main()\n    add(1)\n";
        let info = signature_at(src, 5, 9);
        assert!(info.is_some());
        assert_eq!(info.unwrap().active_param, 0);
    }

    #[test]
    fn test_completions() {
        let items = completions_for_context();
        assert!(!items.is_empty());
        let keywords: Vec<_> = items.iter().filter(|(_, k)| *k == "keyword").collect();
        assert!(keywords.len() > 10);
    }
}
