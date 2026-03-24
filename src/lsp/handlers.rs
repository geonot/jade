use std::collections::HashMap;

use serde_json::Value;

use crate::ast;
use super::analysis::{self, FileAnalysis, SymbolKind, DiagSeverity};
use super::protocol::*;

/// Per-file state: source text + latest analysis.
pub struct ServerState {
    pub files: HashMap<String, String>,
}

impl ServerState {
    pub fn new() -> Self {
        Self { files: HashMap::new() }
    }

    fn analysis_for(&self, uri: &str) -> Option<FileAnalysis> {
        self.files.get(uri).map(|src| analysis::analyze(src))
    }
}

// ── Initialize ─────────────────────────────────────────────────

pub fn handle_initialize(_params: Value) -> Value {
    let result = InitializeResult {
        capabilities: ServerCapabilities {
            text_document_sync: 1, // Full sync
            hover_provider: true,
            definition_provider: true,
            document_symbol_provider: true,
            completion_provider: Some(CompletionOptions {
                trigger_characters: vec![".".into()],
            }),
        },
        server_info: ServerInfo {
            name: "jadec-lsp".into(),
            version: "0.1.0".into(),
        },
    };
    serde_json::to_value(result).unwrap()
}

// ── Document Sync ──────────────────────────────────────────────

pub fn handle_did_open(state: &mut ServerState, params: Value) -> Option<(String, Vec<Diagnostic>)> {
    let p: DidOpenTextDocumentParams = serde_json::from_value(params).ok()?;
    let uri = p.text_document.uri.clone();
    state.files.insert(uri.clone(), p.text_document.text);
    Some((uri.clone(), build_diagnostics(state, &uri)))
}

pub fn handle_did_change(state: &mut ServerState, params: Value) -> Option<(String, Vec<Diagnostic>)> {
    let p: DidChangeTextDocumentParams = serde_json::from_value(params).ok()?;
    let uri = p.text_document.uri.clone();
    if let Some(change) = p.content_changes.into_iter().last() {
        state.files.insert(uri.clone(), change.text);
    }
    Some((uri.clone(), build_diagnostics(state, &uri)))
}

pub fn handle_did_close(state: &mut ServerState, params: Value) {
    if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(params) {
        state.files.remove(&p.text_document.uri);
    }
}

// ── Hover ──────────────────────────────────────────────────────

pub fn handle_hover(state: &ServerState, params: Value) -> Value {
    let p: TextDocumentPositionParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(_) => return Value::Null,
    };
    let src = match state.files.get(&p.text_document.uri) {
        Some(s) => s,
        None => return Value::Null,
    };
    // LSP positions are 0-based; Jade spans are 1-based
    let line1 = p.position.line + 1;
    let col1 = p.position.character + 1;
    let ident = match analysis::find_ident_at(src, line1, col1) {
        Some(id) => id,
        None => return Value::Null,
    };
    let analysis = analysis::analyze(src);
    if let Some((sig, _span)) = analysis.defs.get(&ident) {
        let hover = Hover {
            contents: MarkupContent {
                kind: "markdown",
                value: format!("```jade\n{sig}\n```"),
            },
            range: None,
        };
        return serde_json::to_value(hover).unwrap();
    }
    Value::Null
}

// ── Go to Definition ───────────────────────────────────────────

pub fn handle_definition(state: &ServerState, params: Value) -> Value {
    let p: TextDocumentPositionParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(_) => return Value::Null,
    };
    let src = match state.files.get(&p.text_document.uri) {
        Some(s) => s,
        None => return Value::Null,
    };
    let line1 = p.position.line + 1;
    let col1 = p.position.character + 1;
    let ident = match analysis::find_ident_at(src, line1, col1) {
        Some(id) => id,
        None => return Value::Null,
    };
    let analysis = analysis::analyze(src);
    if let Some((_sig, span)) = analysis.defs.get(&ident) {
        let loc = Location {
            uri: p.text_document.uri,
            range: span_to_range(*span),
        };
        return serde_json::to_value(loc).unwrap();
    }
    Value::Null
}

// ── Document Symbols ───────────────────────────────────────────

pub fn handle_document_symbols(state: &ServerState, params: Value) -> Value {
    let p: DocumentSymbolParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(_) => return Value::Array(vec![]),
    };
    let analysis = match state.analysis_for(&p.text_document.uri) {
        Some(a) => a,
        None => return Value::Array(vec![]),
    };
    let syms: Vec<DocumentSymbol> = analysis
        .symbols
        .iter()
        .map(|s| symbol_to_lsp(s))
        .collect();
    serde_json::to_value(syms).unwrap()
}

// ── Completion ─────────────────────────────────────────────────

pub fn handle_completion(state: &ServerState, params: Value) -> Value {
    let p: TextDocumentPositionParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(_) => return Value::Array(vec![]),
    };
    let mut items: Vec<CompletionItem> = Vec::new();

    // Add keywords/builtins
    for (label, kind_str) in analysis::completions_for_context() {
        items.push(CompletionItem {
            label,
            kind: if kind_str == "function" { CK_FUNCTION } else { CK_KEYWORD },
            detail: None,
        });
    }

    // Add symbols from current file
    if let Some(analysis) = state.analysis_for(&p.text_document.uri) {
        for sym in &analysis.symbols {
            let kind = match sym.kind {
                SymbolKind::Function => CK_FUNCTION,
                SymbolKind::Struct => CK_STRUCT,
                SymbolKind::Enum => CK_ENUM_MEMBER,
                SymbolKind::Field => CK_FIELD,
                SymbolKind::Constant => CK_VARIABLE,
            };
            items.push(CompletionItem {
                label: sym.name.clone(),
                kind,
                detail: Some(sym.detail.clone()),
            });
        }
    }
    serde_json::to_value(items).unwrap()
}

// ── Helpers ────────────────────────────────────────────────────

fn build_diagnostics(state: &ServerState, uri: &str) -> Vec<Diagnostic> {
    let analysis = match state.analysis_for(uri) {
        Some(a) => a,
        None => return Vec::new(),
    };
    analysis
        .diagnostics
        .iter()
        .map(|d| {
            let line0 = d.line.saturating_sub(1);
            let col0 = d.col.saturating_sub(1);
            let end_col0 = if d.end_col > d.col { d.end_col - 1 } else { col0 + 1 };
            Diagnostic {
                range: Range {
                    start: PositionOut { line: line0, character: col0 },
                    end: PositionOut { line: line0, character: end_col0 },
                },
                severity: match d.severity {
                    DiagSeverity::Error => 1,
                    DiagSeverity::Warning => 2,
                },
                message: d.message.clone(),
                source: Some("jadec"),
            }
        })
        .collect()
}

fn span_to_range(span: ast::Span) -> Range {
    let line0 = span.line.saturating_sub(1);
    let col0 = span.col.saturating_sub(1);
    Range {
        start: PositionOut { line: line0, character: col0 },
        end: PositionOut { line: line0, character: col0 + (span.end.saturating_sub(span.start)) as u32 },
    }
}

fn symbol_to_lsp(sym: &analysis::Symbol) -> DocumentSymbol {
    let range = span_to_range(sym.span);
    DocumentSymbol {
        name: sym.name.clone(),
        kind: match sym.kind {
            SymbolKind::Function => SK_FUNCTION,
            SymbolKind::Struct => SK_STRUCT,
            SymbolKind::Enum => SK_ENUM,
            SymbolKind::Field => SK_FIELD,
            SymbolKind::Constant => SK_CONSTANT,
        },
        range: range.clone(),
        selection_range: range,
        children: sym.children.iter().map(|c| symbol_to_lsp(c)).collect(),
    }
}
