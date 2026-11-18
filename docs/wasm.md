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
console.log(api.answer(41)); // 42
console.log(api.greet("hibana"));
```

- `int` は ±(2^53 − 1) の範囲の安全整数を `Number` で受け渡します。`BigInt` は受理しません。接続コードと WASM の入口で有限性・整数性・範囲を検査し、負の0は正の0へ正規化します。
- `number` は有限な `Number` で渡します。WASM の境界では `f64` になり、`NaN` と無限大は拒否します。
- `String` は JavaScript の文字列です。UTF-8 として WASM のメモリへ渡します。
- `bool` は JavaScript の真偽値で渡し、整数の0・1は受け付けません。
- `Vec<T>` は JavaScript の配列、型引数の無い構造体（`struct Point { x: int, y: number }`）はフィールド名をキーにしたオブジェクトです。要素とフィールドは上の型と、入れ子の `Vec` と構造体にできます。接続コードは値を 8 バイトの語の並び（型の番号と中身）にして WASM のメモリへ書き、`runtime.hb` の `__val_in`・`__val_out` が組み立て・読み出します。`Ref<T>`・関数・`enum`・型引数のある構造体は渡せません（コンパイルエラー）。
- 戻り値のない関数は `undefined` を返します。
- `extern "browser" { fn show(message: String) }` などの外部関数は、`imports.hibana_host.show` に同期 JavaScript 関数を指定します。WASM では extern のモジュール文字列ではなく、関数名で接続します。
- ホスト関数も同じ型変換を使います。非同期関数や Promise は扱いません。
- 接続コードは呼び出しごとに一時メモリをリセットします。ホスト関数から同じインスタンスの公開関数へ再入することはできません。

`input()` は `imports.hibana.read(fd, target)` に同期の読み取り関数を渡して使えます。`fd` は標準入力の0、`target` は書き込み先の `Uint8Array` です。書き込んだバイト数を返し、入力終了は0、失敗は負の値にします。指定を省いた Node.js では標準入力を読みます。`put` の出力先は `imports.hibana.print`（省略時は `console.log`）です。

## 現在の対応範囲

整数・`number` の演算、変数、分岐、ループ、文字列、配列、構造体、トレイト、`with` による要求と提供（`exit` を含む）、標準入力に対応し、`examples/first.hb` は4つの実行方式で出力を比較しています。`number` の指数は整数に限り、小数の指数は未対応です。公開関数と外部関数の境界では `int`・`number`・`String`・`bool`・`unit` と、それらの `Vec` と構造体を渡せます。確保したメモリは GC（ガベージコレクション。使わなくなったメモリを自動で回収する仕組み）が回収します。GC は利用者の関数の入口とループの先頭で、前回の GC の後に確保した量が 4 MB か生きている量の大きい方を超えた時に動きます。局所変数はシャドースタック（GC が根として読む、局所変数の写しを置く区画。WASM では文字列定数の後ろの 4 MB）に写します。`__hibana_reset` はヒープとシャドースタックと GC の状態を最初に戻します。

文字列の一時領域はバンプアロケータで確保します。呼び出し終了後は再利用されますが、増えた WASM メモリは縮みません。JS に返した文字列はコピーされ、次の呼び出し後も使えます。生の WASM 関数やポインタを直接呼ぶのではなく、生成された接続コードを使ってください。

## 関連する API と実行コマンド

`hibana::to_web(source, &exports)` は一度の型検査と IR 変換から `WebOutput { wasm, javascript }` を返します。ファイルの保存は呼び出し側で行います。

WASM 単体は `hibana::to_wasm_library(source, &exports)`、接続コード単体は `hibana::to_js_glue(source, &exports)` で生成できます。
