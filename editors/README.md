# エディタ

どちらも `hibana lsp`（Language Server）を起動します。先に `cargo install --path .` で hibana を PATH に入れてください。LSP でできることは次のとおりです。

- エラーの表示（提案と、効果を起こした位置も）
- 整形（`hibana fmt` と同じ）
- hover: 関数・型・構築子・マクロは宣言の形と直前の `//` コメント、それ以外の式は型
- 定義へのジャンプ（同じファイルの中）
- 補完: キーワード・関数・型・構築子・マクロ・変数、`web.` の後はモジュールの名前、`p.` の後は構造体のフィールド。書きかけで構文エラーの間は、最後に解析できた時の名前の表を使う
- 色付け（semantic tokens）: 関数・型・構築子・効果・マクロ・モジュール・フィールドを、名前の表から分けて送る

## VS Code（`vscode/`）

```
cd editors/vscode
npm install
npx @vscode/vsce package     # hibana-0.1.0.vsix ができる
code --install-extension hibana-0.1.0.vsix
```

開発中は、`editors/vscode` を VS Code で開いて F5 で拡張機能の開発用ウィンドウを起動できます。hibana の場所は設定 `hibana.path` で変えられます。色付けは TextMate の文法（`syntaxes/hibana.tmLanguage.json`）です。

## Zed（`zed/`）

semantic tokens は Zed では既定で無効なので、settings.json で有効にします（tree-sitter の色付けの上に重ねる）。

```json
{
  "languages": {
    "hibana": { "semantic_tokens": "combined" }
  }
}
```

コマンドパレットの `zed: install dev extension` で `editors/zed` を選びます（Zed が rustup で拡張機能を wasm にビルドするので、rustup が必要です）。

色付けには tree-sitter の文法 `tree-sitter-hibana/` を使います。Zed は文法を Git から取るので、`extension.toml` の `[grammars.hibana]` の `rev` を、文法を含む push 済みのコミットにします。push せずに試すなら `repository` を `file:///<このリポジトリの絶対パス>` にします。

文法を変えたら `tree-sitter-hibana` で `tree-sitter generate` を実行し、できた `src/` もコミットします（devShell に tree-sitter があります）。
