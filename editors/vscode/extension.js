// VS Code の拡張機能。hibana lsp を起動して、エラー・整形・型の表示・定義へのジャンプを使う
const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const command = vscode.workspace.getConfiguration("hibana").get("path") || "hibana";
  client = new LanguageClient(
    "hibana",
    "hibana",
    { command, args: ["lsp"] },
    { documentSelector: [{ scheme: "file", language: "hibana" }] },
  );
  context.subscriptions.push(client);
  client.start();
}

function deactivate() {
  return client && client.stop();
}

module.exports = { activate, deactivate };
