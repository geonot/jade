use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC base ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Value,
    pub result: Value,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result }
    }
}

#[derive(Serialize)]
pub struct Notification {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: Value,
}

// ── Initialize ─────────────────────────────────────────────────

#[derive(Serialize)]
pub struct ServerCapabilities {
    #[serde(rename = "textDocumentSync")]
    pub text_document_sync: i32,
    #[serde(rename = "hoverProvider")]
    pub hover_provider: bool,
    #[serde(rename = "definitionProvider")]
    pub definition_provider: bool,
    #[serde(rename = "documentSymbolProvider")]
    pub document_symbol_provider: bool,
    #[serde(rename = "completionProvider", skip_serializing_if = "Option::is_none")]
    pub completion_provider: Option<CompletionOptions>,
}

#[derive(Serialize)]
pub struct CompletionOptions {
    #[serde(rename = "triggerCharacters")]
    pub trigger_characters: Vec<String>,
}

#[derive(Serialize)]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
}

#[derive(Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

// ── Text Document Items ────────────────────────────────────────

#[derive(Deserialize)]
pub struct TextDocumentIdentifier {
    pub uri: String,
}

#[derive(Deserialize)]
pub struct VersionedTextDocumentIdentifier {
    pub uri: String,
    pub version: Option<i64>,
}

#[derive(Deserialize)]
pub struct TextDocumentItem {
    pub uri: String,
    #[serde(rename = "languageId")]
    pub language_id: String,
    pub version: i64,
    pub text: String,
}

#[derive(Deserialize)]
pub struct TextDocumentContentChangeEvent {
    pub text: String,
}

#[derive(Deserialize)]
pub struct DidOpenTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentItem,
}

#[derive(Deserialize)]
pub struct DidChangeTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: VersionedTextDocumentIdentifier,
    #[serde(rename = "contentChanges")]
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}

#[derive(Deserialize)]
pub struct DidCloseTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
}

#[derive(Deserialize)]
pub struct TextDocumentPositionParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}

#[derive(Deserialize)]
pub struct DocumentSymbolParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
}

#[derive(Deserialize, Clone, Copy)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

// ── Response types ─────────────────────────────────────────────

#[derive(Serialize)]
pub struct Hover {
    pub contents: MarkupContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
}

#[derive(Serialize)]
pub struct MarkupContent {
    pub kind: &'static str,
    pub value: String,
}

#[derive(Serialize, Clone)]
pub struct Range {
    pub start: PositionOut,
    pub end: PositionOut,
}

#[derive(Serialize, Clone)]
pub struct PositionOut {
    pub line: u32,
    pub character: u32,
}

#[derive(Serialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

#[derive(Serialize)]
pub struct DocumentSymbol {
    pub name: String,
    pub kind: i32,
    pub range: Range,
    #[serde(rename = "selectionRange")]
    pub selection_range: Range,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<DocumentSymbol>,
}

#[derive(Serialize)]
pub struct CompletionItem {
    pub label: String,
    pub kind: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

// ── Diagnostics ────────────────────────────────────────────────

#[derive(Serialize)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
}

#[derive(Serialize)]
pub struct PublishDiagnosticsParams {
    pub uri: String,
    pub diagnostics: Vec<Diagnostic>,
}

// ── Symbol kinds (LSP spec) ────────────────────────────────────

pub const SK_FUNCTION: i32 = 12;
pub const SK_STRUCT: i32 = 23;
pub const SK_ENUM: i32 = 10;
pub const SK_FIELD: i32 = 8;
pub const SK_VARIABLE: i32 = 13;
pub const SK_CONSTANT: i32 = 14;

// Completion item kinds
pub const CK_FUNCTION: i32 = 3;
pub const CK_FIELD: i32 = 5;
pub const CK_VARIABLE: i32 = 6;
pub const CK_KEYWORD: i32 = 14;
pub const CK_STRUCT: i32 = 22;
pub const CK_ENUM_MEMBER: i32 = 20;
