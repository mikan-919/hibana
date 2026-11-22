# hibana

hibana は Rust で実装した静的型付けの言語です。細分化型（条件を満たす値だけを許す型。条件は Z3 で証明します）、`with` による要求と提供（代数的効果）、トレイト、マクロを持ちます。型検査を通ったプログラムは、インタプリタ、JavaScript で実行すると同じ出力になります。

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
    put(sum(v))
    // let w: Vec<Pos> = [1, 0] は 0 > 0 を証明できないので型エラー
}
```

## 機能

- 型: ジェネリクス、細分化型、`with` による要求と提供（代数的効果）、トレイト
- マクロ: `syntax` と `macro` の宣言で文法を拡張する。展開の中で束縛した名前は付け替え、呼び出し側の変数を捕まえない
- 実行: インタプリタ、JavaScript、共通の IR を経由した WebAssembly とネイティブ出力
- 道具: フォーマッタ、テストランナー

## 動かす

Nix の flake で開発環境（Rust、Z3、Node.js）を作ります。

```sh
nix develop -c cargo build
nix develop -c cargo run -q -- examples/first.hb       # 機能を組み合わせた例。色と1桁の整数を入力する
nix develop -c cargo run -q -- tests/programs/types/refine.hb      # インタプリタで実行
nix develop -c cargo run -q -- test tests/programs/stdlib/stdlib.hb # Hibana のテストを実行
nix develop -c cargo test
```

## ソース構成

Rust の実装は `src/` 内で役割ごとに分けています。

- `frontend/`: 字句・構文解析、名前解決、マクロ展開
- `check/`: 型検査と Z3 による証明
- `backend/`: インタプリタ、JavaScript、共通の IR、WebAssembly、ネイティブ出力
- `tooling/`: フォーマッタ
- `cli/`: CLI のコマンド処理と統合的なテスト

`src/lib.rs` と `src/main.rs` が入口で、`pipeline.rs` はコンパイルの各段階、`error.rs` は共通のエラー表現を扱います。標準ライブラリは `lib/` にあります。

## 文書

- [プロジェクト概要](docs/overview.md)
- [入門](docs/tutorial.md)
- [言語仕様](docs/spec.md)
- [開発ロードマップ](docs/roadmap.md)
- [用語集](GLOSSARY.md)
- [WebAssembly](docs/wasm.md)
- [カテゴリ別の例](tests/programs/README.md)
