# WebAssembly を Web から使う

当面の WASM ターゲットは、型検査した関数を JavaScript から呼ぶためのライブラリです。`main` は不要です。既存のインタプリタ・JavaScript ターゲットも引き続き使えます。

## Rust API で生成する

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = std::fs::read_to_string("tests/programs/wasm/wasm_web.hb")?;
    let exports = vec!["echo".into(), "answer".into(), "greet".into()];
    let output = hibana::to_web(&source, &exports)?;
    std::fs::create_dir_all("dist")?;
    std::fs::write("dist/wasm_web.wasm", output.wasm)?;
    std::fs::write("dist/wasm_web.mjs", output.javascript)?;
    Ok(())
}
```

Hibana を依存に追加した Rust プログラムから実行すると、WASM と JavaScript の接続コードを生成できます。ファイルの保存は呼び出し側で行います。公開関数とそこから辿れる関数だけを WASM に含めます。ただしソース全体を型検査します。未対応の構文が呼び出し先に含まれる場合はエラーになります。

ルートディレクトリを HTTP サーバーで配信し、`tests/programs/wasm/wasm_web.html` をブラウザで開くと、文字列を渡してブラウザ側の関数を呼べます。`file://` ではなく HTTP を使います。

## JavaScript との接続

```js
import { instantiate } from "./dist/wasm_web.mjs";

const response = await fetch("./dist/wasm_web.wasm");
if (!response.ok) throw new Error(`HTTP ${response.status}`);
const api = await instantiate(await response.arrayBuffer(), {
  hibana_host: {
    decorate: name => `こんにちは、${name}！`,
    show: message => { console.log(message); },
  },
});
console.log(api.echo("日本語も渡せる"));
console.log(api.answer(41)); // 42n
console.log(api.greet("hibana"));
```

- `int` の引数は安全な整数の `Number` または符号付き 64 ビット範囲の `BigInt`。戻り値は `BigInt` です。
- `str` は JavaScript の文字列です。UTF-8 として WASM のメモリへ渡します。
- 戻り値のない関数は `undefined` を返します。
- `extern "browser" { fn show(message: str) }` などの外部関数は、`imports.hibana_host.show` に同期 JavaScript 関数を指定します。WASM では extern のモジュール文字列ではなく、関数名で接続します。
- ホスト関数も同じ型変換を使います。非同期関数や Promise は扱いません。
- 接続コードは呼び出しごとに一時メモリをリセットします。ホスト関数から同じインスタンスの公開関数へ再入することはできません。

## 現在の対応範囲

整数演算、不変な `let`、関数呼び出し、文字列リテラル・引数・戻り値、外部関数の呼び出しに対応します。文字列連結、可変変数、分岐、ループ、配列、構造体、GC、効果ハンドラーはこの WASM 経路では未対応です。言語全体の機能が WASM で動く段階ではありません。

文字列の一時領域はバンプアロケータで確保します。呼び出し終了後は再利用されますが、増えた WASM メモリは縮みません。JS に返した文字列はコピーされ、次の呼び出し後も使えます。生の WASM 関数やポインタを直接呼ぶのではなく、生成された接続コードを使ってください。

## 関連する API と実行コマンド

`hibana::to_web(source, &exports)` は一度の型検査と IR 変換から `WebOutput { wasm, javascript }` を返します。ファイルの保存は呼び出し側で行います。

WASM 単体は `hibana::to_wasm_library(source, &exports)`、接続コード単体は `hibana::to_js_glue(source, &exports)` で生成できます。
