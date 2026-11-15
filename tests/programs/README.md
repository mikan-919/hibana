# tests/programs

テストの入力を兼ねた例です。利用者が書く正本は `examples/` にあり、ここはテストのために変えて構いません。

言語機能や用途ごとに例を分けています。以下のパスはこのディレクトリからの相対パスです。

- [basics/](basics/): 関数、クロージャ、可変変数、参照、文字列、べき乗などの基本機能
- [types/](types/): 細分化型、構造体、トレイト、等値性、Z3 による証明
- [effects/](effects/): 代数的効果と効果の多相
- [macros/](macros/): 文法拡張、derive、マクロの衛生性
- [stdlib/](stdlib/): 標準ライブラリの使用例とテスト
- [web/](web/): Web ライブラリ、ルーティング、HTTP サーバー
- [interop/](interop/): JavaScript / TypeScript との連携（補助ファイルは `interop/js/`）
- [wasm/](wasm/): WebAssembly とブラウザの連携

## 実行

リポジトリのルートで実行します。

```sh
# インタプリタで実行
nix develop -c cargo run -q -- tests/programs/types/refine.hb

# ファイル内のテストを実行
nix develop -c cargo run -q -- test tests/programs/web/web.hb
```

JavaScript / TypeScript 連携、HTTP サーバー、ブラウザ用の例は実行環境が異なります。各ファイルのコメントと[言語仕様](../docs/spec.md)、[WebAssembly の説明](../docs/wasm.md)を参照してください。
