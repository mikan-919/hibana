# tests/programs

テストの入力を兼ねた例です。利用者が書く正本は `examples/` にあり、ここはテストのために変えて構いません。

言語機能や用途ごとに例を分けています。以下のパスはこのディレクトリからの相対パスです。

- [basics/](basics/): 関数、クロージャ、可変変数、参照、文字列、べき乗、パターン、GC などの基本機能
- [types/](types/): 細分化型、構造体、トレイト、`with` による依存注入、等値性、Z3 による証明
- [effects/](effects/): `with` による要求と提供、要求の多相、非同期
- [macros/](macros/): 文法拡張、マクロの衛生性、.do() を使う雛形のマクロ
- [modules/](modules/): 名前によるファイル読み込み、階層のある名前、依存ファイルの読み込み
- [stdlib/](stdlib/): 標準ライブラリの使用例とテスト
- [interop/](interop/): JavaScript / TypeScript との連携（補助ファイルは `interop/js/`）
- [wasm/](wasm/): WebAssembly とブラウザの連携

## 実行

リポジトリのルートで実行します。

```sh
# インタプリタで実行
nix develop -c cargo run -q -- tests/programs/types/refine.hb

# ファイル内のテストを実行
nix develop -c cargo run -q -- test tests/programs/stdlib/stdlib.hb
```

JavaScript / TypeScript 連携とブラウザ用の例は実行環境が異なります。各ファイルのコメントと[言語仕様](../docs/spec.md)、[WebAssembly の説明](../docs/wasm.md)を参照してください。
