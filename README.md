# hibana

hibana は Rust で実装した静的型付けの言語です。依存型、細分化型（条件を満たす値だけを許す型。条件は Z3 で証明します）、代数的効果、トレイト、マクロを持ちます。型検査を通ったプログラムは、インタプリタ、JavaScript、C のどれで実行しても同じ出力になります。

```
// 条件を満たす値だけを許す型。条件はコンパイル時に証明する
type Pos = int[x -> x > 0]

// 添字 v[i] は 0 <= i && i < len(v) の証明が要る
fn sum(v: Vec<Pos>) -> int {
    let mut s = 0
    let mut i: int[x -> x >= 0] = 0
    while i < len(v) {
        s = s + v[i]
        i = i + 1
    }
    s
}

fn main() {
    let v: Vec<Pos> = [3, 1, 4]
    print(sum(v))
    // let w: Vec<Pos> = [1, 0] は 0 > 0 を証明できないので型エラー
}
```

## 機能

- 型: 依存型、細分化型、代数的効果、トレイト、停止性と正値性の検査
- マクロ: `syntax` と `macro` の宣言で文法を拡張する。展開の中で束縛した名前は付け替え、呼び出し側の変数を捕まえない
- 実行: インタプリタ、JavaScript、C（Boehm GC で回収）、整数 IR を経由した WebAssembly（Wasmtime で実行、ライブラリとして書き出し）
- 道具: フォーマッタ、テストランナー、Language Server、VS Code と Zed の拡張機能、tree-sitter の文法
- パッケージ: Git のパッケージ、npm と Cargo のツールの追加と実行、ロックファイル
- 自己ホスト: hibana で書いた字句解析、構文解析、型検査、JavaScript 生成、フォーマッタ。Rust 版と同じ結果になることをテストで確かめている（実証用で、主な実装は Rust 版）

## 動かす

Nix の flake で開発環境（Rust、Z3、Node.js、tree-sitter）を作ります。

```sh
nix develop -c cargo build
nix develop -c cargo run -q -- tests/programs/types/refine.hb      # インタプリタで実行
nix develop -c cargo run -q -- test                    # Hibana のテストを実行
nix develop -c cargo test
```

## ソース構成

Rust の実装は `src/` 内で役割ごとに分けています。

- `frontend/`: 字句・構文解析、名前解決、マクロ展開、TypeScript の宣言の読み込み
- `check/`: 型検査と Z3 による証明
- `backend/`: インタプリタ、JavaScript、C、整数 IR、WebAssembly
- `tooling/`: フォーマッタ、Language Server、シンボルの索引
- `project/`: ビルド、スクリプト、パッケージ管理（`pkg/`）、外部ツール管理（`tools/`）
- `cli/`: CLI のコマンド処理と統合的なテスト

`src/lib.rs` と `src/main.rs` が入口で、`pipeline.rs` はコンパイルの各段階、`error.rs` は共通のエラー表現を扱います。標準ライブラリは `lib/`、Hibana で書いたコンパイラーは `selfhost/`、エディター連携は `editors/` にあります。

## 文書

- [入門](docs/tutorial.md)
- [言語仕様](docs/spec.md)
- [WebAssembly](docs/wasm.md)
- [カテゴリ別の例](tests/programs/README.md)
