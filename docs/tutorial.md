# hibana 入門

hibana は静的型付けの言語です。型検査を通ったプログラムは、インタプリタ（構文木を直接評価する実行方式）、JavaScript、C のどれでも同じ出力になります。この文書は、書き始めるのに必要な機能を例で順に説明します。言語の規則は `docs/spec.md` にあります。

## 1. 動かす

プロジェクトで繰り返し使うコマンドは、`config.hibana.toml` に書けます。

```toml
[scripts]
build = "hibana build src/main.hb"
dev = "hibana src/main.hb"
```

この例では `hibana build` がビルドのコマンドを、`hibana dev` が実行のコマンドを呼びます。値には任意のシェルコマンドを書けるので、別のビルドツールや開発ツールにも置き換えられます。任意の名前も定義でき、`hibana script <名前>` で呼びます。引数を足す場合は `hibana script <名前> -- 引数...` と書きます。

設定は現在のディレクトリから親へ探し、見つかったファイルのディレクトリで `sh -c` に渡します。コマンドの終了コードも引き継ぎます。パッケージ用の `hibana.toml` とは別の設定です。

`hibana add npm:vite` のように外部ツールを追加すると、`config.hibana.toml` の `[dependencies]` に `"npm:vite" = "*"` が記録され、スクリプト内では `vite build` のようにコマンド名をそのまま使えます。Cargo のツールは `hibana add cargo:名前` で追加します。npm/Cargo はシステムにインストールされているものを使い、取得時にはパッケージのスクリプトやビルドも実行します。

事前の追加なしで実行するには `hibana dlx npm:vite --help`、常用のコマンドとして追加するには `hibana add -g cargo:ripgrep` とします。グローバルの配置先 `${XDG_DATA_HOME:-$HOME/.local/share}/hibana/bin` は自分で `PATH` に追加してください。パッケージと実行コマンドの名前が違っていても扱えます。複数のコマンドから選ぶ場合は `hibana dlx --bin コマンド npm:名前 引数` とします。Hibana 側のオプションは識別子より前に書き、後ろはすべて実行先の引数として渡します。区切りの `--` は省略できます。

Hibana パッケージは `hibana dlx hibana:GitのURL`（または `hibana dlx GitのURL`）で実行します。取得元に `hibana.toml` と `src/main.hb` を用意してください。版を選ぶ場合は `hibana dlx --rev v1.0.0 hibana:GitのURL` とします。依存も共通ストアから読み込み、呼び出し元のプロジェクト設定は変更しません。現状の言語にはコマンドライン引数を読み出す API はありません。

取得したツールもファイル単位の共通ストアを使います。版は `npm:vite@6.1.0` のように指定できます。プロジェクトに追加すると、設定ファイルの隣の `config.hibana.lock.toml` に取得した正確な版を記録します。設定と lock を共有すれば、別環境のスクリプト実行でもその版を取得します。直接のツールの版だけを固定し、npm/Cargo の推移依存までは固定しません。`dlx`・グローバル追加はプロジェクトの lock を使わず初回取得結果を再利用します。自動更新・更新コマンド・削除・GC はまだありません。

ファイルを直接指定する既存コマンドも使えます。

`hibana <ファイル>` でインタプリタが実行します。`hibana js <ファイル>` は JavaScript を標準出力に書き、`hibana build <ファイル>` は C を経由してネイティブの実行ファイルを作ります（`cc` と Boehm GC の `libgc` が要ります）。

```
fn greet(name: str) -> str {
    "hello, " + name
}

fn main() {
    let mut total = 0
    for i in 1..4 { total = total + i }
    print(greet("hibana"))
    print(total)
}
```

出力は `hello, hibana` と `6` です。

- 文の区切りの `;` は省けます。改行が区切りになり、行末が演算子なら次の行へ続きます。
- ブロックの最後の式がその値です。`return` は途中で抜ける時だけ使います。
- `let` で束縛した変数は変更できません。変更する変数は `let mut` で宣言します。
- 整数の `int` は 64 ビットで、範囲を超えると実行時エラーです。ほかの基本の型は `bool`、`str`（文字列）、`unit`（値が無いこと）です。

## 2. 型

`struct` は名前付きのフィールドを持つ型、`enum` は構築子（値を作る名前）の選択肢を持つ型です。`match` は構築子ごとに分岐し、すべての構築子を書かないとエラーになります。

```
use std.*

enum Shape {
    Circle(r: int),
    Rect(w: int, h: int),
}

struct Point { x: int, y: int }

fn area(s: Shape) -> int {
    match s {
        Circle(r) => 3 * r * r
        Rect(w, h) => w * h
    }
}

fn main() {
    let p = Point { x: 1, y: 2 }
    print(p.x + p.y)
    print(area(Rect(3, 4)))
    let xs = map([1, 2, 3], fn(x) { x * 10 })
    print(xs)
    print(unwrap_or(find(xs, fn(x) { x > 15 }), 0))
}
```

出力は `3`、`12`、`[10, 20, 30]`、`20` です。

- 型引数は `<>` で書きます。`Vec<int>` は `int` の配列、`Option<A>` は値が無いかもしれないことを表し、`None` か `Some(値)` です。
- `use std.*` は標準ライブラリ（`lib/std.hb`）の名前を修飾なしで使えるようにします。`map`、`filter`、`fold`、`range`、`join`、`trim` などがあります。一覧は仕様の 10 章にあります。
- `fn(x) { ... }` は無名関数（クロージャ）で、外側の変数を使えます。引数の型は呼び出し先から推論します。

## 3. 検査される条件

型に条件を付けられます。条件を満たすことは、コンパイル時に証明できなければエラーです。証明は組み込みの証明器と、Z3（定理証明器）で行います。

```
fn first(xs: Vec<int>) -> int {
    if len(xs) > 0 { xs[0] } else { 0 }
}

fn half(n: int[x -> x > 0]) -> int { n / 2 }

fn main() {
    print(first([7, 8]))
    print(half(10))
}
```

`xs[0]` は `len(xs) > 0` の分岐の中にあるので、範囲内だと示せます。条件を外すと「添字の範囲を証明できない」というエラーになります。`int[x -> x > 0]` は `x > 0` を満たす `int` の型（細分化型）で、`half(0)` は「条件 false を証明できない」というエラーです。

`mut` の `Vec` も、初期値と `push`・`set_at` の代入から長さを追います。ループと分岐の後は長さが分からなくなるので、添字の前に `if i < len(xs)` で確かめます。

## 4. トレイト・derive・効果

トレイトは、型ごとに実装を書ける関数の組です。`<A: Describe>` は、`A` に `Describe` の実装があることを要求します。実装が無い型を渡すとエラーです。

効果（代数的効果）は、関数が外部に頼む操作です。`effect` で操作を宣言し、使う関数は `! 効果名` で宣言します。`handle` が操作を受け取って処理し、`resume()` で操作の呼び出しの直後から実行を再開します（再開は 1 回だけです）。

```
use std.*

trait Describe<A> {
    fn describe(x: A) -> str
}

impl Describe<int> {
    fn describe(x: int) -> str { "int " + to_str(x) }
}

fn twice<A: Describe>(x: A) -> str { describe(x) + ", " + describe(x) }

derive(Eq, Show)
struct P { x: int, y: int }

effect Log {
    fn log(msg: str)
}

fn work() ! Log {
    log("start")
    log("end")
}

fn main() {
    print(twice(3))
    print(P { x: 1, y: 2 } == P { x: 1, y: 2 })
    print(P { x: 1, y: 2 })
    handle work() {
        log(msg) => { print("log: " + msg); resume() }
    }
}
```

出力は `int 3, int 3`、`true`、`P { x: 1, y: 2 }`、`log: start`、`log: end` です。

- `derive(Eq, Show)` は宣言のマクロで、型の `Eq`（`==`）と `Show`（`to_str` と `print`）の実装を作ります。
- `==` は `Eq` の実装があればそれを使い、無ければ構造の比較をします。

## 5. テスト

`test "名前" { ... }` は `hibana test` が実行します。`assert(条件)` と `assert_eq(結果, 期待値)` で確かめます。ファイルを省くと、`hibana.toml` のあるディレクトリの `src` と `tests` のすべての `.hb` を実行します。

```
use std.*

test "join" {
    assert_eq(join(["a", "b"], "-"), "a-b")
}
```

## 6. 道具

- `hibana fmt <ファイル>` はファイルを整形して書き戻します。設定はありません。
- `hibana lsp` は Language Server を動かし、エディタにエラー表示・整形・hover・定義へのジャンプ・補完・色付けを渡します。VS Code と Zed の拡張機能は `editors/` にあります。
- パッケージは git のリポジトリです。`hibana new 名前` でプロジェクトを作り、`hibana add <git の URL> [tag]` で依存を足します。`hibana fetch` が依存を取ってきて、`config.hibana.lock.toml` にコミットを書きます。パッケージは `${XDG_DATA_HOME:-$HOME/.local/share}/hibana/` でプロジェクト間共有し、同じ内容のファイルは共通ストアに一度だけ保存します。詳細は仕様の 3.6 節です。
- JavaScript と TypeScript のモジュールは `use "./util.ts"` で読めます。型は TypeScript の宣言から読みます。詳細は仕様の 3.4 節です。

## 7. ビルドだけで成果物を作る

マクロは実行時のコードを返すだけでなく、ビルド時に成果物を登録できます。次は `build_web` ライブラリのマクロで WASM・JS 接続コード・静的ファイルをまとめて登録する例です。

```
use build_web

web_build "app.wasm", "app.mjs" source "fn echo(message: str) -> str { message } fn add(a: int, b: int) -> int { a + b }" exports (echo, add)
build_asset "message.txt", "ビルドで生成したファイル"
```

`app.hb` として保存し、次を実行します。

```sh
hibana build app.hb --out-dir dist
```

`dist/app.wasm`、`dist/app.mjs`、`dist/message.txt` ができます。`main` は不要です。`web_build` や `build_asset` の呼び出しは空の宣言列に展開されるので、それ自体の実行時処理は残りません。利用側が成果物名や公開関数を選び、ライブラリが生成の組み合わせを決めます。

生成した JS は次のように使えます（WASM の取得方法は実行環境に合わせます）。

```js
import { instantiate } from "./dist/app.mjs";
const wasm = await (await fetch("./dist/app.wasm")).arrayBuffer();
const app = await instantiate(wasm);
console.log(app.echo("こんにちは"));
console.log(app.add(2n, 3n));
```

生成処理は通常の hibana 関数です。`build_web` は `compile(source, "wasm", exports)` でバイト列を作り、`inspect_exports` で得た関数の型情報から `web_javascript` が JS の文字列を生成します。`artifact(path, bytes)` が成果物を登録する汎用 API で、テキストなら `utf8(text)` でバイト列にします。

自分のマクロからも通常の関数を呼んで生成処理を組み合わせ、最後に `quote {}` を返せます。ビルド時 API を使う関数は実行時には残りません。ファイルはマクロ実行中には書かず、生成・型検査・出力先検査のあとで書き出します。

この例のコンパイル対象は `source` に明示した文字列です。現在のファイルの宣言を自動的に読み取る仕組みではありません。型情報から構造体の受け渡しを自動生成する機能、ビルドの依存グラフ、外部ツールの実行はまだありません。詳しくは仕様の 8.5 節と `lib/build_web.hb`、ブラウザ用の例は `tests/fixtures/build_artifacts.hb` を参照してください。

## 8. 次に読むもの

- 言語の規則と既知の制約: `docs/spec.md`
- 例: `examples/`（効果は `effects.hb`、トレイトは `trait.hb`、Web サーバーは `server.hb`、依存型は `dependent.hb`、マクロは `macros.hb`）
- hibana で書いたコンパイラー: `selfhost/`。字句解析・構文解析・型検査・コード生成・整形があり、自分自身をコンパイルして同じ出力になります（`compile.hb` が入口）。
