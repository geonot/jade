// Jinn VS Code extension entry point.
//
// Launches the in-tree `jinnc-lsp` language server (stdio JSON-RPC) and wires
// it to `.jn` documents, providing diagnostics, hover, go-to-definition,
// document symbols, completion, references, rename, semantic tokens, and
// signature help. Syntax highlighting is contributed declaratively by the
// TextMate grammar and works even when the server is unavailable.

import { existsSync } from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

const LANGUAGE_ID = "jinn-lang";
const SERVER_BIN = process.platform === "win32" ? "jinnc-lsp.exe" : "jinnc-lsp";

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  context.subscriptions.push(
    vscode.commands.registerCommand("jinn.lsp.restart", async () => {
      await stopClient();
      await startClient(context);
    }),
  );

  await startClient(context);
}

export async function deactivate(): Promise<void> {
  await stopClient();
}

async function startClient(context: vscode.ExtensionContext): Promise<void> {
  const config = vscode.workspace.getConfiguration("jinn");
  if (!config.get<boolean>("lsp.enable", true)) {
    return;
  }

  const command = resolveServerPath(config.get<string>("lsp.serverPath", ""));
  if (command === undefined) {
    // No server binary found; syntax highlighting still works. Surface a
    // gentle, dismissible hint rather than failing hard.
    void vscode.window
      .showWarningMessage(
        "Jinn language server (jinnc-lsp) was not found. Language features are disabled; " +
          "syntax highlighting remains active. Build it with `cargo build --release` or set " +
          "`jinn.lsp.serverPath`.",
        "Open Settings",
      )
      .then((choice) => {
        if (choice === "Open Settings") {
          void vscode.commands.executeCommand(
            "workbench.action.openSettings",
            "jinn.lsp.serverPath",
          );
        }
      });
    return;
  }

  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: LANGUAGE_ID }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.jn"),
    },
    outputChannelName: "Jinn Language Server",
  };

  client = new LanguageClient(
    "jinnLsp",
    "Jinn Language Server",
    serverOptions,
    clientOptions,
  );

  context.subscriptions.push(client);
  await client.start();
}

async function stopClient(): Promise<void> {
  if (client === undefined) {
    return;
  }
  const current = client;
  client = undefined;
  await current.stop();
}

/**
 * Resolve the jinnc-lsp executable.
 *
 * Order of precedence:
 *   1. An explicit `jinn.lsp.serverPath` setting (absolute or relative to a
 *      workspace folder).
 *   2. A workspace-local build: `target/release/jinnc-lsp` then
 *      `target/debug/jinnc-lsp` under each workspace folder.
 *   3. `jinnc-lsp` on the system PATH (resolved lazily by the OS at spawn).
 */
function resolveServerPath(configured: string): string | undefined {
  const folders = vscode.workspace.workspaceFolders ?? [];

  if (configured.length > 0) {
    if (path.isAbsolute(configured)) {
      return existsSync(configured) ? configured : undefined;
    }
    for (const folder of folders) {
      const candidate = path.join(folder.uri.fsPath, configured);
      if (existsSync(candidate)) {
        return candidate;
      }
    }
    // Treat a bare name as a PATH lookup.
    return configured;
  }

  for (const folder of folders) {
    for (const profile of ["release", "debug"]) {
      const candidate = path.join(folder.uri.fsPath, "target", profile, SERVER_BIN);
      if (existsSync(candidate)) {
        return candidate;
      }
    }
  }

  // Fall back to PATH; spawn fails gracefully if absent and the client surfaces
  // it on the output channel.
  return SERVER_BIN;
}
