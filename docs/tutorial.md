# hibana 入門

hibana は静的型付けの言語です。型検査を通ったプログラムは、インタプリタ（構文木を直接評価する実行方式）、JavaScript、C のどれでも同じ出力になります。この文書は、書き始めるのに必要な機能を例で順に説明します。言語の規則は `docs/spec.md` にあります。

## 1. 動かす

`hibana <ファイル>` でインタプリタが実行します。`hibana js <ファイル>` は JavaScript を標準出力に書き、`hibana build <ファイル>` は C を経由してネイティブの実行ファイルを作ります（`cc` が要ります）。

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
- パッケージは git のリポジトリです。`hibana new 名前` でプロジェクトを作り、`hibana add <git の URL> [tag]` で依存を足します。`hibana fetch` が依存を取ってきて、`hibana.lock` にコミットを書きます。詳細は仕様の 3.6 節です。
- JavaScript と TypeScript のモジュールは `use "./util.ts"` で読めます。型は TypeScript の宣言から読みます。詳細は仕様の 3.4 節です。

## 7. 次に読むもの

- 言語の規則と既知の制約: `docs/spec.md`
- 例: `examples/`（効果は `effects.hb`、トレイトは `trait.hb`、Web サーバーは `server.hb`、依存型は `dependent.hb`、マクロは `macros.hb`）
- hibana で書いたコンパイラー: `selfhost/`。字句解析・構文解析・型検査・コード生成・整形があり、自分自身をコンパイルして同じ出力になります（`compile.hb` が入口）。
