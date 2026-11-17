# hibana 入門

hibana は静的型付けの言語です。型検査を通ったプログラムは、インタプリタ（構文木を直接評価する実行方式）と JavaScript で同じ出力になります。この文書は、書き始めるのに必要な機能を例で順に説明します。言語の規則は `docs/spec.md` にあります。

## 1. 動かす

ファイルを指定して実行します。

```sh
hibana main.hb
hibana main.hb -- 引数...
```

`hibana <ファイル>` でインタプリタが実行します。JavaScript の生成は Rust API の `hibana::to_js` で行えます。

```
fn greet(name: String) -> String {
    'hello, ' + name
}

fn main() {
    let mut total = 0
    for i in 1..4 { total = total + i }
    put(greet('hibana'))
    put(total)
}
```

出力は `hello, hibana` と `6` です。

- 文の区切りの `;` は省けます。改行が区切りになり、行末が演算子なら次の行へ続きます。
- ブロックの最後の式がその値です。`return` は途中で抜ける時だけ使います。
- `let` で束縛した変数は変更できません。変更する変数は `let mut` で宣言します。
- 整数の `int` は ±(2^53 − 1) の範囲で、演算結果が範囲を超えると実行時エラーです。有限な64ビット浮動小数点数は `number` で表し、`int` はその部分型です。ほかの基本の型は `bool`、`String`（文字列）、`unit`（値が無いこと）です。

## 2. 型

`struct` は名前付きのフィールドを持つ型、`enum` は構築子（値を作る名前）の選択肢を持つ型です。`match` は構築子ごとに分岐し、すべての構築子を書かないとエラーになります。

```
use std -> *

enum Shape {
    Circle(r: int),
    Rect(w: int, h: int),
}

struct Point { x: int, y: int }

fn area(s: Shape) -> int {
    match s {
        Circle(r) -> 3 * r * r
        Rect(w, h) -> w * h
    }
}

fn main() {
    let p = Point { x: 1, y: 2 }
    put(p.x + p.y)
    put(area(Rect(3, 4)))
    let xs = map([1, 2, 3], fn(x) { x * 10 })
    put(xs)
    put(unwrap_or(find(xs, fn(x) { x > 15 }), 0))
}
```

出力は `3`、`12`、`[10, 20, 30]`、`20` です。

- 型引数は `<>` で書きます。`Vec<int>` は `int` の配列、`Option<A>` は値が無いかもしれないことを表し、`None` か `Some(値)` です。
- `use std -> *` は標準ライブラリ（`lib/std.hb`）の名前を修飾なしで使えるようにします。`map`、`filter`、`fold`、`range`、`trim` などの関数と、`xs.join(sep)` などのメソッドがあります。一覧は仕様の 10 章にあります。
- `fn(x) { ... }` は無名関数（クロージャ）で、外側の変数を使えます。引数の型は呼び出し先から推論します。

## 3. 検査される条件

型に条件を付けられます。条件を満たすことは、コンパイル時に証明できなければエラーです。証明は組み込みの証明器と、Z3（定理証明器）で行います。

```
fn first(xs: Vec<int>) -> int {
    if len(xs) > 0 { xs[0] } else { 0 }
}

fn half(n: int[x -> x > 0]) -> int { n / 2 }

fn main() {
    put(first([7, 8]))
    put(half(10))
}
```

`xs[0]` は `len(xs) > 0` の分岐の中にあるので、範囲内だと示せます。条件を外すと「添字の範囲を証明できない」というエラーになります。`int[x -> x > 0]` は `x > 0` を満たす `int` の型（細分化型）で、`half(0)` は「条件 false を証明できない」というエラーです。

`mut` の `Vec` も、初期値と `push`・`set_at` の代入から長さを追います。ループと分岐の後は長さが分からなくなるので、添字の前に `if i < len(xs)` で確かめます。

## 4. トレイトと with

トレイトは、型ごとに実装を書けるメソッドの組です。`<A: Describe>` は、`A` に `Describe` の実装があることを要求します。実装が無い型を渡すとエラーです。

`with` は、呼び出し側の文脈から受け取るものです。関数は `with Log` のようにトレイトを要求し、要求したトレイトのメソッドを `log(msg)` と修飾なしで呼べます（要求を省くと、本体から推論します）。呼ぶ側は `with Log(値) { ... }` で提供します。値は `impl` を持つ値か、メソッドが 1 つのトレイトなら無名関数です。無名関数が値を返すと、呼んだ所から続きが動きます。`exit 値` は `with` ブロックを抜け、ブロックの値をその値にします。

```
use std -> *

trait Describe {
    fn describe(self) -> String
}

impl Describe for int {
    fn describe(self) -> String { 'int ' + self.to_string() }
}

fn twice<A: Describe>(x: A) -> String { x.describe() + ', ' + x.describe() }

trait Log {
    fn log(self, msg: String)
}

fn work() with Log {
    log('start')
    log('end')
}

fn check(n: int) -> int with Fail<String> {
    if n < 0 { fail('negative') } else { n }
}

fn main() {
    put(twice(3))
    with Log(fn(msg) { put('log: ' + msg) }) {
        work()
    }
    let n = with Fail(fn(e) { exit 0 }) { check(-1) }
    put(n)
}
```

出力は `int 3, int 3`、`log: start`、`log: end`、`0` です。`Fail<E>` は std にあり、std の `try(fn() { check(-1) })` は失敗を `Err` にした `Result` を返します。

非同期は組み込みの要求 `Async` です。`Async` を持つ関数を呼ぶと、まだ走っていない値 `Async<T>` になり、`.await` を付けた所で走らせて待ちます。`all` は `Async<T>` の値の `Vec` を、`join` は種類の違う 2 つの値を並行に走らせて合流します。`Async` を持たない関数からは、`spawn` で走らせ始め、`then` で結果を受け取ります。

```
use std -> *

fn fetch(id: int) -> String with Async { 'item' + id.to_string() }

fn load() -> String with Async {
    let first = fetch(1).await          // 待つ
    let rest = all([fetch(2), fetch(3)])  // まだ走らない
    first + ' ' + rest.await.join(' ')  // 並行に走らせて合流する
}

fn on_click() {                         // Async を持たない
    spawn(load()).then(fn(s) { put(s) })
}

fn main() { on_click() }
```

出力は `item1 item2 item3` です。詳しくは仕様の 3.5 節にあります。

- `==` は `Eq` の実装があればそれを使い、無ければ構造の比較をします。

## 5. テスト

`test '名前' { ... }` は `hibana test` が実行します。`assert(条件)` と `assert_eq(結果, 期待値)` で確かめます。実行するファイルを引数で指定します。

```
use std -> *

test 'join' {
    assert_eq(['a', 'b'].join('-'), 'a-b')
}
```

## 6. 道具

- hibana のファイルは `use util` で `util.hb`、`use foo::bar` で `foo/bar.hb` を読みます。経路は読み込み元のファイルのディレクトリを基準にします。`use foo::bar` の関数は `bar.foo()`、`use foo::bar -> {foo}` なら `foo()` と呼びます。
- `hibana fmt <ファイル>` はファイルを整形して書き戻します。設定はありません。
- JavaScript と TypeScript のモジュールは `use './util.ts'` で読めます。型は TypeScript の宣言から読みます。詳細は仕様の 3.4 節です。

## 7. 次に読むもの

- 言語の規則と既知の制約: `docs/spec.md`
- [カテゴリ別の例](../tests/programs/README.md)（with は `tests/programs/effects/with.hb`、トレイトは `tests/programs/types/trait.hb`、マクロは `tests/programs/macros/macros.hb`）
