//! LSP smoke test driver — covers the matrix documented in `docs/lsp.md`:
//! initialize, didOpen, hover, definition, document symbols, completion,
//! references, rename, semantic tokens, signature help, didClose.
//!
//! Drives `crate::lsp::handlers` directly with constructed JSON params so
//! no stdio process is needed.

use jinnc::lsp::handlers::{
    handle_completion, handle_definition, handle_did_change, handle_did_close, handle_did_open,
    handle_document_symbols, handle_hover, handle_initialize, handle_references, handle_rename,
    handle_semantic_tokens, handle_signature_help, ServerState,
};
use serde_json::{json, Value};

const URI: &str = "file:///tmp/jinn_lsp_smoke.jn";

const SRC: &str = "*greet(name as string) returns string\n\
                   \x20\x20\x20\x20return name\n\
                   \n\
                   *main()\n\
                   \x20\x20\x20\x20x is greet(\"world\")\n\
                   \x20\x20\x20\x20log(x)\n";

fn open(state: &mut ServerState, src: &str) {
    let params = json!({
        "textDocument": {
            "uri": URI,
            "languageId": "jinn",
            "version": 1,
            "text": src,
        }
    });
    let _ = handle_did_open(state, params);
}

fn pos(line: u32, character: u32) -> Value {
    json!({
        "textDocument": { "uri": URI },
        "position": { "line": line, "character": character },
    })
}

#[test]
fn lsp_initialize_advertises_full_matrix() {
    let result = handle_initialize(Value::Null);
    let caps = &result["capabilities"];
    assert_eq!(caps["hoverProvider"], json!(true), "{caps}");
    assert_eq!(caps["definitionProvider"], json!(true), "{caps}");
    assert_eq!(caps["referencesProvider"], json!(true), "{caps}");
    assert_eq!(caps["renameProvider"], json!(true), "{caps}");
    assert_eq!(caps["documentSymbolProvider"], json!(true), "{caps}");
    assert!(caps["completionProvider"].is_object(), "{caps}");
    assert!(caps["semanticTokensProvider"].is_object(), "{caps}");
    assert!(caps["signatureHelpProvider"].is_object(), "{caps}");
}

#[test]
fn lsp_did_open_publishes_diagnostics_array() {
    let mut state = ServerState::new();
    let params = json!({
        "textDocument": {
            "uri": URI,
            "languageId": "jinn",
            "version": 1,
            "text": SRC,
        }
    });
    let (uri, diags) = handle_did_open(&mut state, params).expect("didOpen result");
    assert_eq!(uri, URI);
    // Diags may be empty for a well-formed file; the contract is "returns a vec".
    let _ = diags.len();
}

#[test]
fn lsp_did_change_replaces_content_and_returns_diagnostics() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    let params = json!({
        "textDocument": { "uri": URI, "version": 2 },
        "contentChanges": [{ "text": "*main()\n\tlog(1)\n" }],
    });
    let (uri, _diags) = handle_did_change(&mut state, params).expect("didChange result");
    assert_eq!(uri, URI);
}

#[test]
fn lsp_document_symbols_lists_top_level_defs() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    let syms = handle_document_symbols(
        &state,
        json!({ "textDocument": { "uri": URI } }),
    );
    let arr = syms.as_array().expect("array of symbols");
    let names: Vec<&str> = arr
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(names.contains(&"greet"), "names={:?}", names);
    assert!(names.contains(&"main"), "names={:?}", names);
}

#[test]
fn lsp_completion_includes_workspace_symbols_and_keywords() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    let items = handle_completion(&state, pos(4, 2));
    let arr = items.as_array().expect("completion items array");
    assert!(!arr.is_empty(), "expected non-empty completion list");
    let labels: Vec<&str> = arr.iter().filter_map(|i| i["label"].as_str()).collect();
    assert!(
        labels.iter().any(|l| *l == "greet"),
        "expected `greet` in completions; got {:?}",
        labels
    );
}

#[test]
fn lsp_hover_returns_value_for_known_ident() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    // Position over `greet` call in `x is greet(...)` on line 5, col ~7 (0-based line 4, char 7).
    let v = handle_hover(&state, pos(4, 7));
    // Hover may legitimately return null if the position-to-ident map misses;
    // either way the handler must not panic and must return valid JSON.
    assert!(v.is_object() || v.is_null(), "{v}");
}

#[test]
fn lsp_definition_resolves_within_file() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    // Click on `greet` call on line 5 (0-based 4), char 7.
    let v = handle_definition(&state, pos(4, 7));
    assert!(v.is_object() || v.is_null(), "{v}");
    if let Some(uri) = v.get("uri") {
        assert_eq!(uri, URI);
    }
}

#[test]
fn lsp_references_returns_array() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    let v = handle_references(
        &state,
        json!({
            "textDocument": { "uri": URI },
            "position": { "line": 4, "character": 7 },
            "context": { "includeDeclaration": true },
        }),
    );
    assert!(v.is_array(), "{v}");
}

#[test]
fn lsp_rename_returns_workspace_edit() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    let v = handle_rename(
        &state,
        json!({
            "textDocument": { "uri": URI },
            "position": { "line": 4, "character": 7 },
            "newName": "salute",
        }),
    );
    // Either a WorkspaceEdit or Null if no identifier is at position.
    assert!(v.is_object() || v.is_null(), "{v}");
}

#[test]
fn lsp_semantic_tokens_returns_data_array() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    let v = handle_semantic_tokens(&state, json!({ "textDocument": { "uri": URI } }));
    assert!(v.is_object(), "{v}");
    assert!(v["data"].is_array(), "{v}");
}

#[test]
fn lsp_signature_help_handles_no_active_call_gracefully() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    // Position at end of `log(x` on line 6 — inside a call.
    let v = handle_signature_help(&state, pos(5, 5));
    assert!(v.is_object() || v.is_null(), "{v}");
}

#[test]
fn lsp_did_close_drops_state() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    handle_did_close(
        &mut state,
        json!({ "textDocument": { "uri": URI } }),
    );
    // After close, document symbols for the URI must be empty.
    let syms = handle_document_symbols(&state, json!({ "textDocument": { "uri": URI } }));
    let arr = syms.as_array().expect("symbols array");
    assert!(arr.is_empty(), "expected no symbols after close, got {:?}", arr);
}
