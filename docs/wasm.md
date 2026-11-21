# WebAssembly を Web から使う

当面の WASM ターゲットは、型検査した関数を JavaScript から呼ぶためのライブラリです。`main` は不要です。既存のインタプリタ・JavaScript ターゲットも引き続き使えます。

## hibana のビルドスクリプトで生成する

```
use std::build
use std::fs
use std::wasm_web

fn main() {
    let source = 'tests/programs/wasm/wasm_web.hb'
    let compiled = build::compile_wasm(source, ['echo', 'answer', 'greet'])
    let abi = compiled.second
    fs::write_bytes('dist/wasm_web.wasm', compiled.first)
    fs::write_bytes('dist/wasm_web.mjs', wasm_web::utf8(wasm_web::javascript(abi)))
    fs::write_bytes('dist/wasm_web.d.mts', wasm_web::utf8(wasm_web::typescript(abi)))
}
```

上のコードを `build.hb` に保存し、リポジトリのルートで `hibana build.hb` を実行します。`build::compile_wasm` は一度のコンパイルで WASM のバイト列と受け渡し規約（ABI）の JSON を返します。`std::wasm_web` はその規約から JavaScript の接続コードと TypeScript の型宣言を生成します。スクリプトが保存先を指定して `fs::write_bytes` で書き込みます。経路はスクリプトのディレクトリから数えます。WASM には公開関数とそこから辿れる関数が含まれます。

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

## WASM の受け渡し規約

接続コードを書くライブラリは、`build::compile_wasm` が返す JSON の `exports` と `imports` を使います。各関数は `[名前, 引数の型の並び, 戻り値の型]` です。取り込み先のモジュール名は外部関数が `hibana_host`、標準入出力と時刻が `hibana` です。

- 基本型は `int`、`number`、`String`、`bool`、`unit` の文字列です。`int` と `number` は WASM の `f64`、`bool` は `i64` の 0 または 1 です。`unit` の戻り値はありません。
- `Vec` は `{ "vec": 要素型 }`、構造体は `{ "ctor": 構築子番号, "fields": [[名前, 型], ...] }`、`Result` は `{ "result": [[成功の構築子番号, 型], [失敗の構築子番号, 型]] }` です。
- `String` と複合値は `i64` の上位32ビットにバイト数、下位32ビットに `memory` 内の位置を入れます。書き込み先は `__hibana_alloc` で確保します。複合値はリトルエンディアン（下位バイトを先に置く順序）の64ビット語の並びです。`unit` は `[0]`、`int`・`bool`・`number` は `[1]`・`[2]`・`[9]` と値の語、`String` は `[3, バイト数]` と8バイト境界に揃えた UTF-8、`Vec` は `[4, 要素数]` と要素の並び、構造体と `Result` は `[5, 構築子番号, 項目数]` と項目の並びです。
- 実行時エラーで WASM が停止した場合、`memory` の `failure_offset` にメッセージの位置が入ります。0 はメッセージ無しを表します。メッセージの位置には64ビットの長さがあり、続けて UTF-8 の本文があります。呼び出し後に `__hibana_reset` が公開されていれば、状態と一時メモリを初期化できます。

Rust から生成する場合は `hibana::compile_wasm_library(source, &exports)` がバイト列と受け渡し規約を返します。変換済みの IR があれば `pipeline::emit_wasm` と `pipeline::wasm_abi` を使えます。
