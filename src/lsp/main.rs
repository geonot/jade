use std::io::{self, BufReader, Write};

use serde_json::Value;

use jadec::lsp::handlers::{self, ServerState};
use jadec::lsp::protocol::{Notification, PublishDiagnosticsParams, Response};
use jadec::lsp::transport;

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    let mut state = ServerState::new();
    let mut initialized = false;

    loop {
        let msg = match transport::read_message(&mut reader) {
            Ok(Some(m)) => m,
            Ok(None) => break, // EOF
            Err(e) => {
                eprintln!("jadec-lsp: read error: {e}");
                break;
            }
        };

        let req: jadec::lsp::protocol::Request = match serde_json::from_str(&msg) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("jadec-lsp: invalid JSON-RPC: {e}");
                continue;
            }
        };

        match req.method.as_str() {
            "initialize" => {
                let result = handlers::handle_initialize(req.params);
                send_response(&mut writer, req.id.unwrap_or(Value::Null), result);
                initialized = true;
            }
            "initialized" => {
                // Client ack — nothing to do
            }
            "shutdown" => {
                send_response(&mut writer, req.id.unwrap_or(Value::Null), Value::Null);
            }
            "exit" => {
                break;
            }
            "textDocument/didOpen" if initialized => {
                if let Some((uri, diags)) = handlers::handle_did_open(&mut state, req.params) {
                    publish_diagnostics(&mut writer, &uri, diags);
                }
            }
            "textDocument/didChange" if initialized => {
                if let Some((uri, diags)) = handlers::handle_did_change(&mut state, req.params) {
                    publish_diagnostics(&mut writer, &uri, diags);
                }
            }
            "textDocument/didClose" if initialized => {
                handlers::handle_did_close(&mut state, req.params);
            }
            "textDocument/hover" if initialized => {
                let result = handlers::handle_hover(&state, req.params);
                send_response(&mut writer, req.id.unwrap_or(Value::Null), result);
            }
            "textDocument/definition" if initialized => {
                let result = handlers::handle_definition(&state, req.params);
                send_response(&mut writer, req.id.unwrap_or(Value::Null), result);
            }
            "textDocument/documentSymbol" if initialized => {
                let result = handlers::handle_document_symbols(&state, req.params);
                send_response(&mut writer, req.id.unwrap_or(Value::Null), result);
            }
            "textDocument/completion" if initialized => {
                let result = handlers::handle_completion(&state, req.params);
                send_response(&mut writer, req.id.unwrap_or(Value::Null), result);
            }
            _ => {
                // Unknown or notification — ignore
                if let Some(id) = req.id {
                    // Request needs a response — send method-not-found
                    let err = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": "method not found" }
                    });
                    let body = serde_json::to_string(&err).unwrap();
                    let _ = transport::write_message(&mut writer, &body);
                }
            }
        }
    }
}

fn send_response(writer: &mut impl Write, id: Value, result: Value) {
    let resp = Response::ok(id, result);
    let body = serde_json::to_string(&resp).unwrap();
    let _ = transport::write_message(writer, &body);
}

fn publish_diagnostics(writer: &mut impl Write, uri: &str, diagnostics: Vec<jadec::lsp::protocol::Diagnostic>) {
    let params = PublishDiagnosticsParams {
        uri: uri.to_string(),
        diagnostics,
    };
    let notif = Notification {
        jsonrpc: "2.0",
        method: "textDocument/publishDiagnostics",
        params: serde_json::to_value(params).unwrap(),
    };
    let body = serde_json::to_string(&notif).unwrap();
    let _ = transport::write_message(writer, &body);
}
