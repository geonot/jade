//! LSP request/notification handlers (textDocument/* and workspace/*).

use std::collections::HashMap;

use serde_json::Value;

use super::analysis::{self, DiagSeverity, FileAnalysis, SymbolKind};
use super::protocol::*;
use crate::ast;

pub struct ServerState {
    pub files: HashMap<String, String>,
    pub workspace_index: HashMap<String, Vec<WorkspaceSymbol>>,
}

pub struct WorkspaceSymbol {
    pub name: String,
    pub sig: String,
    pub uri: String,
    pub span: ast::Span,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            workspace_index: HashMap::new(),
        }
    }

    fn analysis_for(&self, uri: &str) -> Option<FileAnalysis> {
        self.files.get(uri).map(|src| analysis::analyze(src))
    }

    fn update_index(&mut self, uri: &str) {
        let analysis = match self.analysis_for(uri) {
            Some(a) => a,
            None => return,
        };
        let mut syms = Vec::new();
        for (name, (sig, span)) in &analysis.defs {
            syms.push(WorkspaceSymbol {
                name: name.clone(),
                sig: sig.clone(),
                uri: uri.to_string(),
                span: *span,
            });
        }
        self.workspace_index.insert(uri.to_string(), syms);
    }

    fn find_in_workspace(&self, name: &str) -> Option<&WorkspaceSymbol> {
        for syms in self.workspace_index.values() {
            for sym in syms {
                if sym.name == name {
                    return Some(sym);
                }
            }
        }
        None
    }
}

pub fn handle_initialize(_params: Value) -> Value {
    let result = InitializeResult {
        capabilities: ServerCapabilities {
            text_document_sync: 1,
            hover_provider: true,
            definition_provider: true,
            document_symbol_provider: true,
            completion_provider: Some(CompletionOptions {
                trigger_characters: vec![".".into()],
            }),
            references_provider: true,
            rename_provider: true,
            semantic_tokens_provider: Some(SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: vec![
                        "keyword",
                        "function",
                        "variable",
                        "string",
                        "number",
                        "operator",
                        "type",
                        "comment",
                        "enumMember",
                    ],
                    token_modifiers: vec![],
                },
                full: true,
            }),
            signature_help_provider: Some(SignatureHelpOptions {
                trigger_characters: vec!["(".into(), ",".into()],
            }),
        },
        server_info: ServerInfo {
            name: "jinnc-lsp".into(),
            version: "0.2.0".into(),
        },
    };
    serde_json::to_value(result).expect("ICE: LSP serialization")
}

pub fn handle_did_open(
    state: &mut ServerState,
    params: Value,
) -> Option<(String, Vec<Diagnostic>)> {
    let p: DidOpenTextDocumentParams = serde_json::from_value(params).ok()?;
    let uri = p.text_document.uri.clone();
    state.files.insert(uri.clone(), p.text_document.text);
    state.update_index(&uri);
    Some((uri.clone(), build_diagnostics(state, &uri)))
}

pub fn handle_did_change(
    state: &mut ServerState,
    params: Value,
) -> Option<(String, Vec<Diagnostic>)> {
    let p: DidChangeTextDocumentParams = serde_json::from_value(params).ok()?;
    let uri = p.text_document.uri.clone();
    if let Some(change) = p.content_changes.into_iter().last() {
        state.files.insert(uri.clone(), change.text);
    }
    state.update_index(&uri);
    Some((uri.clone(), build_diagnostics(state, &uri)))
}

pub fn handle_did_close(state: &mut ServerState, params: Value) {
    if let Ok(p) = serde_json::from_value::<DidCloseTextDocumentParams>(params) {
        state.files.remove(&p.text_document.uri);
    }
}

pub fn handle_hover(state: &ServerState, params: Value) -> Value {
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
    if let Some((sig, _span)) = analysis.defs.get(&ident) {
        let hover = Hover {
            contents: MarkupContent {
                kind: "markdown",
                value: format!("```jinn\n{sig}\n```"),
            },
            range: None,
        };
        return serde_json::to_value(hover).expect("ICE: LSP serialization");
    }
    Value::Null
}

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
    // Try same-file first
    let analysis = analysis::analyze(src);
    if let Some((_sig, span)) = analysis.defs.get(&ident) {
        let loc = Location {
            uri: p.text_document.uri,
            range: span_to_range(*span),
        };
        return serde_json::to_value(loc).expect("ICE: LSP serialization");
    }
    // Try cross-file workspace index
    if let Some(ws) = state.find_in_workspace(&ident) {
        let loc = Location {
            uri: ws.uri.clone(),
            range: span_to_range(ws.span),
        };
        return serde_json::to_value(loc).expect("ICE: LSP serialization");
    }
    Value::Null
}

pub fn handle_document_symbols(state: &ServerState, params: Value) -> Value {
    let p: DocumentSymbolParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(_) => return Value::Array(vec![]),
    };
    let analysis = match state.analysis_for(&p.text_document.uri) {
        Some(a) => a,
        None => return Value::Array(vec![]),
    };
    let syms: Vec<DocumentSymbol> = analysis.symbols.iter().map(|s| symbol_to_lsp(s)).collect();
    serde_json::to_value(syms).expect("ICE: LSP serialization")
}

pub fn handle_completion(state: &ServerState, params: Value) -> Value {
    let p: TextDocumentPositionParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(_) => return Value::Array(vec![]),
    };
    let mut items: Vec<CompletionItem> = Vec::new();

    for (label, kind_str) in analysis::completions_for_context() {
        items.push(CompletionItem {
            label,
            kind: if kind_str == "function" {
                CK_FUNCTION
            } else {
                CK_KEYWORD
            },
            detail: None,
        });
    }

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
    serde_json::to_value(items).expect("ICE: LSP serialization")
}

// ── References handler ─────────────────────────────────────────

pub fn handle_references(state: &ServerState, params: Value) -> Value {
    let p: ReferenceParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(_) => return Value::Array(vec![]),
    };
    let src = match state.files.get(&p.text_document.uri) {
        Some(s) => s,
        None => return Value::Array(vec![]),
    };
    let line1 = p.position.line + 1;
    let col1 = p.position.character + 1;
    let ident = match analysis::find_ident_at(src, line1, col1) {
        Some(id) => id,
        None => return Value::Array(vec![]),
    };

    let mut locations: Vec<Location> = Vec::new();

    // Same-file references
    for r in analysis::find_references(src, &ident) {
        locations.push(Location {
            uri: p.text_document.uri.clone(),
            range: Range {
                start: PositionOut {
                    line: r.line.saturating_sub(1),
                    character: r.col.saturating_sub(1),
                },
                end: PositionOut {
                    line: r.line.saturating_sub(1),
                    character: r.col.saturating_sub(1) + r.len,
                },
            },
        });
    }

    // Cross-file references
    for (uri, src) in &state.files {
        if *uri == p.text_document.uri {
            continue;
        }
        for r in analysis::find_references(src, &ident) {
            locations.push(Location {
                uri: uri.clone(),
                range: Range {
                    start: PositionOut {
                        line: r.line.saturating_sub(1),
                        character: r.col.saturating_sub(1),
                    },
                    end: PositionOut {
                        line: r.line.saturating_sub(1),
                        character: r.col.saturating_sub(1) + r.len,
                    },
                },
            });
        }
    }

    serde_json::to_value(locations).expect("ICE: LSP serialization")
}

// ── Rename handler ─────────────────────────────────────────────

pub fn handle_rename(state: &ServerState, params: Value) -> Value {
    let p: RenameParams = match serde_json::from_value(params) {
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

    let mut changes: HashMap<String, Vec<TextEdit>> = HashMap::new();

    // Same-file edits
    let refs = analysis::find_references(src, &ident);
    if !refs.is_empty() {
        let edits: Vec<TextEdit> = refs
            .iter()
            .map(|r| TextEdit {
                range: Range {
                    start: PositionOut {
                        line: r.line.saturating_sub(1),
                        character: r.col.saturating_sub(1),
                    },
                    end: PositionOut {
                        line: r.line.saturating_sub(1),
                        character: r.col.saturating_sub(1) + r.len,
                    },
                },
                new_text: p.new_name.clone(),
            })
            .collect();
        changes.insert(p.text_document.uri.clone(), edits);
    }

    // Cross-file edits
    for (uri, file_src) in &state.files {
        if *uri == p.text_document.uri {
            continue;
        }
        let refs = analysis::find_references(file_src, &ident);
        if !refs.is_empty() {
            let edits: Vec<TextEdit> = refs
                .iter()
                .map(|r| TextEdit {
                    range: Range {
                        start: PositionOut {
                            line: r.line.saturating_sub(1),
                            character: r.col.saturating_sub(1),
                        },
                        end: PositionOut {
                            line: r.line.saturating_sub(1),
                            character: r.col.saturating_sub(1) + r.len,
                        },
                    },
                    new_text: p.new_name.clone(),
                })
                .collect();
            changes.insert(uri.clone(), edits);
        }
    }

    let edit = WorkspaceEdit { changes };
    serde_json::to_value(edit).expect("ICE: LSP serialization")
}

// ── Semantic tokens handler ────────────────────────────────────

pub fn handle_semantic_tokens(state: &ServerState, params: Value) -> Value {
    let p: DocumentSymbolParams = match serde_json::from_value(params) {
        Ok(p) => p,
        Err(_) => return Value::Null,
    };
    let src = match state.files.get(&p.text_document.uri) {
        Some(s) => s,
        None => return Value::Null,
    };
    let tokens = analysis::semantic_tokens(src);
    let mut data = Vec::with_capacity(tokens.len() * 5);
    for t in &tokens {
        data.push(t.delta_line);
        data.push(t.delta_start);
        data.push(t.length);
        data.push(t.token_type);
        data.push(t.token_modifiers);
    }
    let result = SemanticTokensResult { data };
    serde_json::to_value(result).expect("ICE: LSP serialization")
}

// ── Signature help handler ─────────────────────────────────────

pub fn handle_signature_help(state: &ServerState, params: Value) -> Value {
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
    let info = match analysis::signature_at(src, line1, col1) {
        Some(i) => i,
        None => return Value::Null,
    };
    let sig_help = SignatureHelp {
        signatures: vec![SignatureInformation {
            label: info.label,
            parameters: info
                .params
                .iter()
                .map(|p| ParameterInformation { label: p.clone() })
                .collect(),
        }],
        active_signature: 0,
        active_parameter: info.active_param,
    };
    serde_json::to_value(sig_help).expect("ICE: LSP serialization")
}

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
            let end_col0 = if d.end_col > d.col {
                d.end_col - 1
            } else {
                col0 + 1
            };
            Diagnostic {
                range: Range {
                    start: PositionOut {
                        line: line0,
                        character: col0,
                    },
                    end: PositionOut {
                        line: line0,
                        character: end_col0,
                    },
                },
                severity: match d.severity {
                    DiagSeverity::Error => 1,
                    DiagSeverity::Warning => 2,
                },
                message: d.message.clone(),
                source: Some("jinnc"),
            }
        })
        .collect()
}

fn span_to_range(span: ast::Span) -> Range {
    let line0 = span.line.saturating_sub(1);
    let col0 = span.col.saturating_sub(1);
    Range {
        start: PositionOut {
            line: line0,
            character: col0,
        },
        end: PositionOut {
            line: line0,
            character: col0 + (span.end.saturating_sub(span.start)) as u32,
        },
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
