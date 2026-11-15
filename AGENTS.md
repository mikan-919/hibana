# AGENTS.md

hibana は Rust で実装した静的型付けの言語です。依存型・細分化型（Z3 で証明）・代数的効果・トレイト・マクロを持ち、インタプリタ、JavaScript、WebAssembly、ネイティブのバックエンドがあります。言語の規則は `docs/spec.md`、入門は `docs/tutorial.md` です。

## 開発環境

Nix の flake で開発環境を作ります（Rust のツールチェーン、Z3、Node.js、tree-sitter）。コマンドは `nix develop -c ...` で実行します。

```sh
nix develop -c cargo build            # target/debug/hibana を作る
nix develop -c cargo test             # 全テスト（約 10 秒）
nix develop -c cargo run -q -- <ファイル>   # 実行（fmt / test / lsp は第 1 引数）
```

- `cargo test` は `target/debug/hibana` を作り直しません。CLI を試す前に `cargo build` を実行します。
- `cargo test <名前>` で絞れます（例: `cargo test builtin_prover`）。

## 構成

| 場所 | 内容 |
|---|---|
| `src/frontend/` | 字句解析（`lexer.rs`）、構文解析（`parser.rs`）、名前解決（`resolve.rs`）、マクロ展開（`meta.rs`）、TypeScript の宣言の読み込み（`ts.rs`） |
| `src/check/` | 型検査（`typeck.rs`）。依存型の評価、細分化型の証明、停止性・正値性の検査、効果、トレイト。`smt.rs` は Z3 の FFI |
| `src/backend/` | インタプリタ（`eval.rs`）、JavaScript（`js.rs`）、WASM とネイティブの共通の構造化 IR（`ir.rs`）、WebAssembly（`wasm.rs`）、Cranelift によるネイティブ（`native.rs`。libc なしの静的 ELF） |
| `src/tooling/` | フォーマッタ（`fmt.rs`）、Language Server（`lsp.rs`）、シンボルの索引（`index.rs`） |
| `src/project/` | ビルド（`build.rs`）、スクリプト（`scripts.rs`）、パッケージ管理（`pkg/`）、外部ツール管理（`tools/`） |
| `src/cli/mod.rs` | CLI のコマンド処理と、全体のテスト（`mod tests`） |
| `src/lib.rs` `src/main.rs` | ライブラリと CLI の入口。既存の `hibana::parser` などの API は再エクスポートで維持 |
| `src/pipeline.rs` `src/error.rs` | コンパイルの各段階と、共通のエラー表現 |
| `lib/` | 標準ライブラリ（`std.hb`）と web ライブラリ（`web.hb`）。hibana で書いてある |
| `examples/` | 利用者が書く正本。動作と書き方の基準。エージェントは書き換えない |
| `tests/programs/` | カテゴリ別の例（一覧は `tests/programs/README.md`）。テストの入力になる |
| `editors/` | VS Code・Zed の拡張機能と tree-sitter の文法 |

## 規約

- インタプリタ・JavaScript・WASM・ネイティブの出力をテストで比較します。バックエンドに手を入れたら対応するテストを実行します。
- 組み込み関数を足す時は `src/check/typeck.rs` の `BUILTINS`、`src/frontend/parser.rs` の `globals`、各バックエンド、`docs/spec.md` の 9 章を更新します。`std` に足した名前が組み込み（`all`、`resume` など）と衝突しないか確認します。
- `lib/std.hb` に関数を足したら `tests/programs/stdlib/stdlib.hb` にテストを足します。
- 簡略化して上限がある所には `ponytail:` で始まるコメントに、上限と改善の方向を書きます。
- コメント・エラーメッセージ・ドキュメントは日本語です。修飾表現（「非常に」など）は使わず、事実を書きます。
- `examples/` は利用者が書く正本です。実装の動作と書き方はこれに合わせます。`examples/` のファイルは利用者の指示なしに作成・変更・削除しません。正本と実装が食い違う時は、実装を直すか、利用者に報告します。`x.hb` の横に `x.out` があれば、インタプリタの出力がその中身と同じかをテストが確かめます。
- `.hb` のファイルは `hibana fmt` で整形します。`lib/`・`tests/programs/`・`examples/` は整形済みで、テストが確かめます。

## Git

- 動作確認が済んだ作業単位ごとにコミットします。
- 追跡済みファイルの移動・削除は `git mv`・`git rm` で行います。
- コミットメッセージは英語で、変更の内容を 1 行目に書きます。
