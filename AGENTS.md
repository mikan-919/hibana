# AGENTS.md

hibana は Rust で実装した静的型付けの言語です。依存型・細分化型（Z3 で証明）・代数的効果・トレイト・マクロを持ち、インタプリタ、JavaScript、C のバックエンドがあります。言語の規則は `docs/spec.md`、入門は `docs/tutorial.md` です。

## 開発環境

Nix の flake で開発環境を作ります（Rust のツールチェーン、Z3、Node.js、tree-sitter）。コマンドは `nix develop -c ...` で実行します。

```sh
nix develop -c cargo build            # target/debug/hibana を作る
nix develop -c cargo test             # 全テスト（約 10 秒）
nix develop -c cargo run -q -- <ファイル>   # 実行（js / c / build / fmt / test / lsp は第 1 引数）
```

- `cargo test` は `target/debug/hibana` を作り直しません。CLI を試す前に `cargo build` を実行します。
- `cargo test <名前>` で絞れます（例: `cargo test selfhost_codegen`）。

## 構成

| 場所 | 内容 |
|---|---|
| `src/lexer.rs` `src/parser.rs` | 字句解析と構文解析。名前の解決は `resolve.rs`、TypeScript の宣言の読み込みは `ts.rs` |
| `src/meta.rs` | マクロ（`syntax`・`macro`・宣言のマクロ）の展開 |
| `src/typeck.rs` | 型検査。依存型の評価（`Norm`）、細分化型の証明、停止性・正値性の検査、効果、トレイト |
| `src/smt.rs` | Z3 の FFI。証明器は Z3 なしでも多くの条件を示せる |
| `src/eval.rs` `src/js.rs` `src/c.rs` | インタプリタ、JavaScript、C のバックエンド。3 つは同じ出力になる |
| `src/fmt.rs` `src/lsp.rs` `src/index.rs` | フォーマッタ（設定なし）、Language Server、シンボルの索引 |
| `src/pkg.rs` | パッケージ管理（`hibana.toml`・`hibana.lock`） |
| `src/main.rs` | CLI と、全体のテスト（`mod tests`） |
| `lib/` | 標準ライブラリ（`std.hb`）と web ライブラリ（`web.hb`）。hibana で書いてある |
| `examples/` | 例。多くはテストの入力になる |
| `selfhost/` | hibana で書いたコンパイラー。機能の実証用で、本流は Rust の実装 |
| `editors/` | VS Code・Zed の拡張機能と tree-sitter の文法 |

## 規約

- 新しい言語機能は先に Rust 版へ入れます。selfhost に Rust 版の全機能を移す予定はありません（実証用です）。
- インタプリタ・JavaScript・C は同じ出力になることをテストで確かめています。バックエンドに手を入れたら、3 つとも確認します。
- 組み込み関数を足す時は `typeck.rs` の `BUILTINS`、`parser.rs` の `globals`、3 つのバックエンド、`docs/spec.md` の 9 章を更新します。`std` に足した名前が組み込み（`all`、`resume` など）と衝突しないか確認します。
- `lib/std.hb` に関数を足したら `examples/stdlib.hb` にテストを足します。`std` を読む selfhost のコンパイラーも通る必要があります。
- 簡略化して上限がある所には `ponytail:` で始まるコメントに、上限と改善の方向を書きます。
- コメント・エラーメッセージ・ドキュメントは日本語です。修飾表現（「非常に」など）は使わず、事実を書きます。
- `.hb` のファイルは `hibana fmt` で整形します。`lib/` と `examples/` は整形済みで、テストが確かめます。

## selfhost のテスト

`src/main.rs` の `selfhost_*` テストが次を確かめます。

- 字句（`selfhost_lexer_matches`）と構文木（`selfhost_parser_matches`）が Rust 版と同じ。
- 型検査（`selfhost_checker_matches`）が Rust 版と同じく通すか止める。
- 生成した JavaScript の出力がインタプリタと同じ（`selfhost_codegen_matches`）。自分自身をコンパイルした 2 段目と 3 段目の出力が同じ（固定点）であることも確かめます。
- 整形（`selfhost_fmt_matches`）が Rust 版と同じ。

selfhost の `.hb` を変えたら `cargo test selfhost_` を実行します。selfhost のコードは、添字の範囲を証明できる形（`if i < len(xs)` の保護、`tk()` のような安全な取り出し関数）で書きます。相互再帰は `partial fn` にします。

## Git

- 動作確認が済んだ作業単位ごとにコミットします。
- 追跡済みファイルの移動・削除は `git mv`・`git rm` で行います。
- コミットメッセージは英語で、変更の内容を 1 行目に書きます。
