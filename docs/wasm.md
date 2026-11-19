# WebAssembly を Web から使う

当面の WASM ターゲットは、型検査した関数を JavaScript から呼ぶためのライブラリです。`main` は不要です。既存のインタプリタ・JavaScript ターゲットも引き続き使えます。

## hibana のビルドスクリプトで生成する

```
fn main() {
    let source = 'tests/programs/wasm/wasm_web.hb'
    let exports = ['echo', 'answer', 'greet']
    write_bytes('dist/wasm_web.wasm', compile_web_file(source, 'wasm', exports))
    write_bytes('dist/wasm_web.mjs', compile_web_file(source, 'js', exports))
    write_bytes('dist/wasm_web.d.mts', compile_web_file(source, 'ts', exports))
}
```

上のコードを `build.hb` に保存し、リポジトリのルートで `hibana build.hb` を実行します。`compile_web_file` は公開する関数名を受け取り、`'wasm'` で WebAssembly ライブラリ、`'js'` で JavaScript の接続コード、`'ts'` で TypeScript の型宣言のバイト列を返します。`write_bytes` が保存先のディレクトリを作って書き込みます。WASM には公開関数とそこから辿れる関数が含まれます。ソース全体の型検査を行い、呼び出し先に未対応の構文が含まれるとエラーになります。

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
- `Vec<T>` は JavaScript の配列、型引数の無い構造体（`struct Point { x: int, y: number }`）はフィールド名をキーにしたオブジェクトです。要素とフィールドは上の型と、入れ子の `Vec` と構造体にできます。
- 公開関数の戻り値には標準ライブラリの `Result<T, E>` も使えます。`Ok(value)` は `{ ok: true, value }` に、`Err(error)` は `{ ok: false, error }` になります。`T` と `E` には戻り値として渡せる型を使い、配列の要素や構造体のフィールドにも `Result` を含められます。
- 接続コードは複合値を 8 バイトの語の並び（型の番号と中身）で受け渡します。`Ref<T>`・関数・標準の `Result` 以外の列挙型・型引数のある構造体を境界に指定するとコンパイルエラーになります。
- 戻り値のない関数は `undefined` を返します。
- `with Async` の公開関数は、WASM 内の仕事の並びが空になるまで実行し、完了した値を同期で返します。JavaScript からは `api.twice(40)` のように呼び出します。
- `extern "browser" { fn show(message: String) }` などの外部関数は、`imports.hibana_host.show` に同期 JavaScript 関数を指定します。WASM では extern のモジュール文字列ではなく、関数名で接続します。
- ホスト関数も同じ型変換を使います。非同期関数や Promise は扱いません。
- 公開関数の引数とホスト関数の戻り値に細分化型（値に条件を付けた型）を指定するとコンパイルエラーになります。配列の要素と構造体のフィールドも対象です。`int` などで受け取り、hibana 側で `refine<T>` を使うと条件を検査できます。
- 接続コードは呼び出しごとに一時メモリをリセットします。ホスト関数から同じインスタンスの公開関数へ再入することはできません。

`input()` は `imports.hibana.read(fd, target)` に同期の読み取り関数を渡して使えます。`fd` は標準入力の0、`target` は書き込み先の `Uint8Array` です。書き込んだバイト数を返し、入力終了は0、失敗は負の値にします。指定を省いた Node.js では標準入力を読みます。`put` の出力先は `imports.hibana.print`（省略時は `console.log`）です。

`spawn` した処理の実行時エラーは、`imports.hibana.error`（省略時は `console.error`）にメッセージを渡し、残りの処理を続けます。同期の `main`、非同期の公開関数、または `then` に渡した関数が失敗した場合は、残りの処理を動かしてから、公開関数の呼び出しを例外で終えます。公開関数と `then` に渡した関数の両方が失敗した場合は、公開関数のエラーを例外にします。

## 現在の対応範囲

整数・`number` の演算、変数、分岐、ループ、文字列、配列、構造体、トレイト、`with` による要求と提供（`exit` を含む）、標準入力に対応し、`examples/first.hb` は4つの実行方式で出力を比較しています。`number` の指数は整数値に限り、小数部分のある指数は実行時エラーです。公開関数と外部関数の境界では `int`・`number`・`String`・`bool`・`unit` と、それらの `Vec` と構造体を渡せます。確保したメモリは GC（ガベージコレクション。使わなくなったメモリを自動で回収する仕組み）が回収します。GC は利用者の関数の入口とループの先頭で、前回の GC の後に確保した量が 4 MB か生きている量の大きい方を超えた時に動きます。局所変数はシャドースタック（GC が根として読む、局所変数の写しを置く区画。WASM では文字列定数の後ろの 4 MB）に写します。`__hibana_reset` はヒープ、シャドースタック、GC、要求の処理、非同期の仕事の状態を最初に戻します。

文字列の一時領域はバンプアロケータで確保します。呼び出し終了後は再利用されますが、増えた WASM メモリは縮みません。JS に返した文字列はコピーされ、次の呼び出し後も使えます。生の WASM 関数やポインタを直接呼ぶのではなく、生成された接続コードを使ってください。

## Rust から使う API

`hibana::to_web(source, &exports)` は一度の型検査と IR 変換から `WebOutput { wasm, javascript }` を返します。ファイルの保存は呼び出し側で行います。

WASM 単体は `hibana::to_wasm_library(source, &exports)`、接続コード単体は `hibana::to_js_glue(source, &exports)` で生成できます。

`hibana::to_ts_declarations(source, &exports)` は、同じ公開関数を指定した接続コードに対応する TypeScript の型宣言を文字列で返します。`wasm_web.mjs` と組み合わせる場合は `wasm_web.d.mts` に保存します。公開関数、戻り値、呼び出しに必要なホスト関数の型が宣言に含まれます。`Result` の戻り値は `ok` の真偽に応じて `value` と `error` の型が決まります。

型検査と IR 変換を済ませている場合は、同じ IR と公開関数名を `pipeline::emit_ts_declarations` に渡せます。
