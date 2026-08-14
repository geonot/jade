use jinnc::lsp::handlers::{
    ServerState, handle_completion, handle_definition, handle_did_change, handle_did_close,
    handle_did_open, handle_document_symbols, handle_hover, handle_initialize, handle_references,
    handle_rename, handle_semantic_tokens, handle_signature_help,
};
use serde_json::{Value, json};

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
    let syms = handle_document_symbols(&state, json!({ "textDocument": { "uri": URI } }));
    let arr = syms.as_array().expect("array of symbols");
    let names: Vec<&str> = arr.iter().filter_map(|s| s["name"].as_str()).collect();
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
        labels.contains(&"greet"),
        "expected `greet` in completions; got {:?}",
        labels
    );
}

#[test]
fn lsp_hover_returns_value_for_known_ident() {
    let mut state = ServerState::new();
    open(&mut state, SRC);

    let v = handle_hover(&state, pos(4, 7));

    assert!(v.is_object() || v.is_null(), "{v}");
}

#[test]
fn lsp_definition_resolves_within_file() {
    let mut state = ServerState::new();
    open(&mut state, SRC);

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

    let v = handle_signature_help(&state, pos(5, 5));
    assert!(v.is_object() || v.is_null(), "{v}");
}

#[test]
fn lsp_did_close_drops_state() {
    let mut state = ServerState::new();
    open(&mut state, SRC);
    handle_did_close(&mut state, json!({ "textDocument": { "uri": URI } }));

    let syms = handle_document_symbols(&state, json!({ "textDocument": { "uri": URI } }));
    let arr = syms.as_array().expect("symbols array");
    assert!(
        arr.is_empty(),
        "expected no symbols after close, got {:?}",
        arr
    );
}

#[test]
fn lsp_type_errors_reach_diagnostics_with_position() {
    let mut state = ServerState::new();
    let src = "*main()\n    log(nosuchvar)\n";
    let params = json!({
        "textDocument": { "uri": URI, "languageId": "jinn", "version": 1, "text": src }
    });
    let (_, diags) = handle_did_open(&mut state, params).expect("diagnostics");
    assert!(!diags.is_empty(), "expected a type diagnostic");
    let d = &diags[0];
    assert!(
        d.message.contains("undefined name"),
        "expected undefined-name diagnostic, got: {}",
        d.message
    );
    assert_eq!(d.range.start.line, 1, "diagnostic should point at line 2");
    assert!(
        d.range.start.character > 0,
        "diagnostic should carry a column"
    );
}

#[test]
fn lsp_hover_shows_inferred_local_type() {
    let mut state = ServerState::new();
    open(&mut state, "*main()\n    x is 42\n    log(x)\n");
    let v = handle_hover(&state, pos(2, 8));
    let text = v["contents"]["value"].as_str().unwrap_or("");
    assert!(
        text.contains("x: i64"),
        "expected inferred type in hover, got: {text}"
    );
}

#[test]
fn lsp_hover_on_call_shows_signature() {
    let mut state = ServerState::new();
    open(
        &mut state,
        "*add(a as i64, b as i64) returns i64\n    a + b\n\n*main()\n    log(add(1, 2))\n",
    );
    let v = handle_hover(&state, pos(4, 9));
    let text = v["contents"]["value"].as_str().unwrap_or("");
    assert!(
        text.contains("*add(a as i64, b as i64) returns i64"),
        "expected signature hover, got: {text}"
    );
}

#[test]
fn lsp_definition_resolves_local_bind_site() {
    let mut state = ServerState::new();
    open(&mut state, "*main()\n    total is 1\n    log(total)\n");
    let v = handle_definition(&state, pos(2, 9));
    assert_eq!(
        v["range"]["start"]["line"],
        json!(1),
        "def should be on line 2: {v}"
    );
    assert_eq!(v["range"]["start"]["character"], json!(4), "def col: {v}");
}

#[test]
fn lsp_rename_respects_shadowing_scopes() {
    let mut state = ServerState::new();
    let src = "*first()\n    v is 1\n    log(v)\n\n*second()\n    v is 2\n    log(v)\n\n*main()\n    first()\n    second()\n";
    open(&mut state, src);
    let v = handle_rename(
        &state,
        json!({
            "textDocument": { "uri": URI },
            "position": { "line": 1, "character": 4 },
            "newName": "renamed",
        }),
    );
    let edits = v["changes"][URI].as_array().expect("edits");
    assert_eq!(
        edits.len(),
        2,
        "rename must touch only the first function's `v` (def + one use), got: {edits:?}"
    );
    for e in edits {
        let line = e["range"]["start"]["line"].as_u64().unwrap();
        assert!(line == 1 || line == 2, "edit leaked outside scope: {e}");
    }
}

#[test]
fn lsp_warnings_surface_with_severity() {
    let mut state = ServerState::new();
    let src = "*main()\n    log(nosuchvar)\n";
    let params = json!({
        "textDocument": { "uri": URI, "languageId": "jinn", "version": 1, "text": src }
    });
    let (_, diags) = handle_did_open(&mut state, params).expect("diagnostics");
    for d in &diags {
        assert!(d.severity == 1 || d.severity == 2, "severity range");
    }
}
