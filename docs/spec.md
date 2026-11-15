# hibana 言語仕様

hibana は、細分化型と、構文を拡張できるマクロを根幹に置く言語です。実装は Rust で、`nix develop` の中で `cargo run -- <ファイル.hb>` で実行します。証明に使う Z3 は、z3 クレートでライブラリ（libz3）を直接呼びます。`nix develop` でビルドした実行ファイルは libz3 の場所を rpath に持つので、外でもそのまま動きます。`cargo build --features static` は Z3 をソースからビルドして静的にリンクします（配布用。時間がかかり、cmake が要ります）。

この文書は実装（`src/`）の現状を書いたものです。例は `tests/programs/` にあります。利用者が書く正本は `examples/` です。

## 1. 字句

- 識別子は英字か `_` で始まり、英数字と `_` が続きます。
- 整数（`42`、`0xFF`）と文字列（`'a\n'`）を書けます。文字列は `'...'` だけで書き、`"` は使えません。浮動小数（`3.14`）は字句解析だけ対応していて、式には書けません（マクロのパターンの `Float` で使えます）。文字列のエスケープは `\n \t \r \0 \\ \' \` \{ \}` です。
- `` `Hello {name}!` `` は埋め込み付きの文字列です。`{ }` の中に式を1つ書き、その値を `to_str` で文字列にしてつなぎます（`'Hello ' + to_str(name) + '!'` と同じ）。`{ }` の中には `'...'` や入れ子の `` `...` `` も書けます。
- `型の名前::名前`（`Response::text`）は1つの名前です（3.9 節）。
- コメントは `// 行末まで` と `/* ... */` です。
- キーワード: `fn let mut if else while for in return match type struct enum trait impl syntax macro quote use module extern effect handle true false`
- 各トークンは、直前に空白があるか（`gap_before`）と改行があるか（`nl_before`）を持ちます。前者はマクロのパターンの `~`（空白なしで続く）に、後者は文の区切りに使います。

## 2. 文の区切り

文は改行で終わります。`;` を書いても構いません。行頭の二項演算子・`(`・`.`・`[` は前の行に続かず、新しい文の始まりになります。式を複数行に分ける時は、演算子を行末に置きます。

## 3. 宣言

ファイルの最上位には `use`・`fn`・`type`・`effect`・`syntax`・`macro` を並べます。宣言の順序は自由で、後で宣言した型や関数・マクロも使えます。`main` という引数の無い関数が入口です。

### 3.1 関数

```
fn add(x: int, y: int) -> int { x + y }
fn map<A, B>(f: (A) -> B, l: List<A>) -> List<B> { ... }
```

- 戻り値の型を省くと `unit`（値の無いこと）です。この場合、本体の最後の式の値は捨てます。
- 本体の最後の式が戻り値です。途中で返す時は `return` を使います。
- `<A, B>` はジェネリック引数です（`<A>` は `<A: Type>` の省略形）。呼ぶ側では省くことができ、引数の型と期待する型から推論します。明示する時は `map<int, int>(f, l)` と書きます。
- 関数は型を値として受け取ったり返したりできません。型引数は `fn id<A>(x: A) -> A` のように宣言します。

### 3.2 型

型の宣言は3種類です。`struct` と `enum` は実行時に値として残る形を、`type` は型検査のためだけの型（型の別名）を宣言します。

```
// 構造体。値は Point { x: 1, y: 2 } で作り、p.x で取り出す
struct Point { x: int, y: int }

// フィールドに名前の無い構造体。int とは別の型になる。値は UserId(42) で作り、id.0 で取り出す
struct UserId(int)

// 直和型。構築子は , か改行で区切る
enum Nat { Zero, Succ(n: Nat) }
enum Option<T> { None, Some(x: T) }

// 別名。型引数（`<A>`）を取れる。実行時には消える
type Pos = int[x -> x > 0]
```

- 型の引数は `<>` で宣言し、`<>` で渡します（`Option<int>`）。値の引数は `()` で渡します（`Some(3)`）。型の名前に `()` で引数を渡すとエラーです（`struct Name(型)` の構造体の値を作る時を除く）。
- 構築子の型引数は推論します。`let o: Option<int> = None` のように期待する型から、`Some(3)` のように引数の型から求めます。
- 構造体の値のフィールドの順序は自由です。構造体は、構築子を1つ持つ型として扱います。`struct Name(型, ...)` のフィールドの名前は `0`、`1`、... で、`Name(1, 2)` と表示します。
- `type Name = { ... }` と `type Name = A | B` はエラーで、`struct` と `enum` を使うよう伝えます。
- 名前の決まり: 組み込みのプリミティブ型は小文字（`int bool str syn unit`）、それ以外の型は大文字で始めます。

### 3.3 モジュール

```
use web              // 組み込みのライブラリ（lib/ 以下。実行ファイルに埋め込まれている）。web.ok のように修飾して使う
use web -> *            // すべての名前を修飾なしで使う
use web -> {ok, Request}  // 挙げた名前だけ修飾なしで使う
use 'util.hb'        // ファイル（今のディレクトリからの経路）。モジュール名は拡張子を除いた名前（util）
use './lib' -> *     // ./ か ../ で始まる経路は .hb を省ける。-> * はすべての名前を修飾なしで使う
use './lib'
    -> { hanabi }    // -> { 名前 } は挙げた名前だけ修飾なしで使う。改行して続けて書ける
    -> { Response }  // 型の名前を挙げると、その型に属する関数（Response::text など）も修飾なしで使える
```

- `use` したモジュールの関数・型・構築子は `モジュール名.名前`（`web.ok('a')`、`web.Request { ... }`、`Option<web.Request>`）で参照します。利用者のファイルの同じ名前とは衝突しません。
- モジュールの中では、そのモジュール自身の名前を修飾なしで参照できます。
- 名前は、局所変数、自分のモジュール、`use` で修飾なしに取り込んだ名前の順に探します。
- `syntax` と `macro` は、`use` したモジュールのものもすべて有効です（パターンのリテラルで呼び出すため）。マクロの展開結果がライブラリの関数や型を使っていても、衛生性によって定義した側のモジュールの名前（`web.not_found` など）を指すので、利用者が修飾なしで取り込んでいなくても動きます。
- 同じモジュールは1回だけ読みます。別の経路で書いた同じ名前のファイル（`'./lib'` と `'../web/lib'`）も1回です。
- ファイル名がキーワードと同じ（`syntax.hb`）なら、モジュール名の末尾に `_` を足します（`syntax_`）。

### 3.4 JavaScript / TypeScript のモジュール

JS ターゲット（Rust API の `hibana::to_js`）でだけ使えます。インタプリタで呼ぶと実行時エラーになります。

```
use './js/util.ts'        // TypeScript のファイル。export した関数・interface・type の型を読む
use './js/greet.js'       // JavaScript のファイル。隣の greet.d.ts の型を読む
use 'some-package'        // npm のパッケージ。node_modules の package.json の types、index.d.ts、@types/... を探す
use 'node:path'           // Node.js の組み込み。node_modules/@types/node が要る

fn main() {
    put(util.add(2, 3))
    let p = util.point(1, 2)       // interface Point は構造体 util.Point になる
    put(p.x + p.y)
}
```

- `use` に名前で書く（`use web`）と組み込みのライブラリ、文字列で書くとファイルかパッケージです。ファイルの経路は、`use` を書いた .hb のファイルのディレクトリからの経路です。
- モジュール名は、ファイル名から拡張子を除いた名前です（`./js/util.ts` は `util`、`node:path` は `path`、名前に使えない文字は `_`）。hibana のモジュールと同じく、`use './js/util.ts' -> *` で修飾なしにもできます。
- 型は TypeScript の型宣言から読み、境界でその型に従って値を変換します。型宣言の無い JavaScript は使えません。

| TypeScript の型 | hibana の型 | 境界での変換 |
|---|---|---|
| `number` | `int` | number と BigInt（整数でなければ実行時エラー） |
| `string` `boolean` `void` | `str` `bool` `unit` | そのまま |
| `T[]` `Array<T>` | `Vec<T>` | 要素ごと |
| `(x: A) => B` | `(A) -> B` | 引数と戻り値を変換するよう包む |
| `interface P { ... }`、`type P = { ... }`、オブジェクト型 | 構造体 | 同じフィールド名のオブジェクト |
| `Promise<T>`（戻り値） | `T` と `! Async`（3.5 節） | 呼び出しを `await` する |
| `(x: A) => Promise<B>` | `(A) -> B ! Async` | hibana の関数は Promise を返す関数として渡す |
| 共用体・ジェネリクス・`any` などそれ以外 | `js` | JavaScript の値をそのまま持ち回る |

- ジェネリクスや可変長引数を持つ関数、メソッドは読めないので無視します。
- 内部では、型宣言から `extern '経路' { fn 名前(引数) -> 型 }` の宣言を作って使います。型宣言の無いモジュールには、この `extern` を直接書くこともできます。

### 3.5 効果

非同期・失敗・依存の注入などは、代数的エフェクト（効果の「操作」を呼ぶと、外側の `handle` の腕がそれを処理し、必要なら呼んだ所から続きを再開する仕組み）で書きます。Promise や `await` は書きません。例は `tests/programs/effects/effects.hb` です。

```
effect Fail<E> { fn fail(e: E) -> never }
effect Need<T> { fn need() -> T }

fn find(id: str) -> User ! Fail<NotFound>, Need<Db> {
    let db = need()
    if id == '0' { fail(NotFound { id: id }) } else { User { name: db.prefix + id } }
}

fn main() {
    let name = handle find('42').name {
        fail(e) => 'not found: ' + e.id       // resume しない腕は handle 全体の値になる
        need() => resume(Db { prefix: 'u' })  // resume で、操作の戻り値を渡して続きを再開する
    }
}
```

- `effect Name<引数> { fn 操作(引数) -> 型 ... }` で効果を宣言します。効果は構築子の無い型で、操作は効果の引数をジェネリック引数に持つ関数です。戻らない操作の戻り値の型は `never` と書きます。操作でない関数も戻り値の型を `never` にできますが、本体が `panic` などの戻らない式で終わり、`return` を含まないことを型検査で確かめます。
- 関数が起こす効果は、戻り値の型の後ろに `! 効果, ...` と書きます。書いていない効果を起こすと型エラーです。効果を起こす関数を呼ぶ関数も、その効果を書くか `handle` で処理する必要があります。
- `handle 式 { 操作(引数) => 式 ... }` は、式の中で起きた腕の操作を処理します。処理した効果は、`handle` の外の型から消えます。効果の型引数（`Fail<E>` の `E`）は、本体で起こした効果の型から決まります。
- 腕の中の操作は、その `handle` より外側で処理します（深いハンドラー）。
- 関数型にも効果を書けます（`(str) -> unit ! Log`）。期待する型が効果を持つ関数型なら、無名関数の中でその効果を起こせます。起こす効果の少ない関数は、多い関数型の値として使えます（効果の無い関数を `! Log` の引数に渡せる）。
- 効果の多相: `<E: Effect>` は効果の並びを表す型引数です。`fn map<A, B, E: Effect>(xs: Vec<A>, f: (A) -> B ! E) -> Vec<B> ! E` を呼ぶと、`E` は渡した関数の効果になり（効果の無い関数なら空）、`map` もその効果を起こします。関数の中では `E` を1つの効果として扱い、`E` の値の関数を呼べます。std の `map` `filter` `fold` がこの形です（例は `tests/programs/effects/poly.hb`）。
  - 渡す無名関数の引数の型は、前の引数から決まった関数型の引数の型から推論します（`map([1, 2], fn(x) { x * 2 })`）。戻り値の型と効果は本体から求め、`E` は本体で起きた効果になります。書けば、書いた型と効果を使います。
  - 効果の変数に `Async` も入れられます。JavaScript では、効果の多相な関数ごとに、効果の変数を通した呼び出しを待つ async 版（`f_map$a`）も出力し、`E` に `Async` が入る呼び出しはそちらを呼んで待ちます（async かどうかだけの単相化）。
- 引数の並びの中で、効果を持つ関数型の後に名前の無い引数が続く時は、関数型を括弧で囲みます（`(int, ((int) -> int ! Log), str) -> int`）。
- `resume(値)`（unit なら `resume()`）は続きを1回だけ再開します。腕の最後（腕の値になる位置）にだけ書けます。戻り値の型が `never` の操作は再開できません。
- `main` が起こせる効果は `Async` だけです。
- `all(fs)` は、`() -> T ! Async` の `Vec` の関数を並行に実行し、結果を `Vec<T>` で返します（`Async` の効果を起こす）。`all([fn() { ... }, ...])` と書けば、無名関数の中で `Async` を起こせます。すべての処理が終わるまで `all` から戻りません（構造化並行性）。インタプリタでは順に実行します。
- JavaScript では、処理中の handle の並びを作り直しで更新し、`await` の後に待つ前の並びへ戻すので、並行に動く処理の handle は混ざりません。

`Async` は組み込みの効果で、待つ処理を表します。TypeScript の `Promise<T>` を返す関数は、`T` を返して `Async` の効果を起こす関数として読みます（3.4 節）。

```
use './js/util.ts'        // export async function later(x: number): Promise<number>

fn twice_later(x: int) -> int ! Async {
    util.later(x) + util.later(x)       // await は書かない
}

fn main() ! Async { put(twice_later(5)) }
```

実装:

| 実行方式 | 操作の呼び出し | resume しない腕 | Async |
|---|---|---|---|
| インタプリタ | 処理中の handle の並びから、一番内側の腕を呼ぶ | 抜け出す理由（`Flow::Abort`）として handle まで戻す | 意味を持たない |
| JavaScript | `$op` が同じように腕を呼ぶ | 例外（`$Abort`）で handle の `try` まで戻す | `Async` を持つ関数を `async function` にし、呼び出しを `await` する |
| WASM・ネイティブ（IR） | ランタイムの `__perform` が同じように腕を呼ぶ | 中断の印を置き、効果を持つ関数・関数の値の呼び出しの後で調べて 1 段ずつ戻す。handle の本体は `Catch` で抜ける | 意味を持たない（`all` は順に実行する） |

制限:

- `resume` は腕の最後でだけ使えます。続きの後に処理を書く（`resume` の結果を使う）には、継続を実装する必要があります。
- 期待する型の無い無名関数（`let f = fn(x: int) { ... }`）は効果を起こせません。効果の推論はまだしません。
- 効果の操作そのものは値として使えません（`let f = log` は不可。`fn(m) { log(m) }` と包む）。
- `handle` の中では `return` を使えません。
- 腕の中では `Async` を起こせません（腕は同期的に呼びます）。
- `all` は、失敗した処理があっても残りを中断せず、すべて終わるのを待ってから最初の失敗を投げます。

### 3.6 パッケージ

パッケージは git のリポジトリで、入口は `src/lib.hb` です。プロジェクトの直下の `hibana.toml` に依存を書くと、`use 名前` で使えます。名前は組み込みのライブラリ（`std` `web`）、`hibana.toml` の依存の順に探します。

```
[package]
name = "app"
version = "0.1.0"

[dependencies]
greet = { git = "https://github.com/someone/hibana-greet", tag = "v0.1.0" }   # tag の代わりに branch か rev も書ける。無ければ既定のブランチ
```

- `hibana fetch` は依存（依存の依存も）を `${XDG_DATA_HOME:-$HOME/.local/share}/hibana/` に保存し、`config.hibana.lock.toml` にコミットの SHA を書きます。`XDG_DATA_HOME` は絶対パスのみ有効です。ロックにある依存はそのコミットを使います。版を上げるときは `config.hibana.lock.toml` から消して取り直します。
- ファイルは内容の SHA-256 と実行属性で識別して共通ストアに一度だけ保存し、パッケージのツリーへハードリンクします。同じ内容・実行属性なら、プロジェクト・版・取得元をまたいで共有します。プロジェクトにはコピーせず、共通のツリーを直接参照します。保存したファイルは読み取り専用です。
- 取得用の git 履歴は一時領域だけに置き、保存時に `.git` を除きます。パッケージ内に収まる相対シンボリックリンクのみ使えます。ハードリンクできない環境ではコピーせずエラーにします。旧 `XDG_CACHE_HOME` のキャッシュは移行・削除せず、新しい保存先に再取得します。ストアの自動削除・GC はまだありません。
- `hibana.toml` のあるディレクトリの下のファイルを実行・変換すると、取ってきていない依存を先に取ってきます。
- パッケージの中の `use 'x.hb'` は、そのファイルのディレクトリから探します。
- 同じ名前のパッケージは一つの版だけ使えます。違う git の URL で二つ要るとエラーです。版の範囲（semver）の解決とレジストリはまだありません。
- JavaScript のパッケージは npm で入れます（3.4 節）。

### 3.7 トレイト

トレイトは、型ごとに実装を書ける関数の組です（例は `tests/programs/types/trait.hb`）。

```
trait Show<A> {
    fn show(x: A) -> str
    fn loud(x: A) -> str { show(x) + '!' }   // 既定の実装。impl で省ける
}

impl Show<int> {
    fn show(x: int) -> str { to_str(x) }
}

impl<A: Show> Show<Vec<A>> {                 // 条件付きの実装
    fn show(xs: Vec<A>) -> str { ... }
}

fn twice<A: Show>(x: A) -> str { show(x) + show(x) }   // 制約。<A: Show + Eq> で複数

trait Ord<A>: Eq<A> {                        // 親のトレイト。Ord の実装には Eq の実装が要る
    fn lt(a: A, b: A) -> bool
}
```

- メソッドは、ふつうの関数として `show(x)` で呼びます。
- 仕組みは辞書渡し（dictionary passing）です。`trait Show<A>` は構造体の型 `Show<A>`（メソッドの関数をフィールドに持つ）になり、`impl` はその型の値を作る関数になります。制約 `<A: Show>` は、実装を受け取る隠れた引数です。呼び出す側では、型検査が型引数を求めた後に実装を探して、その引数を足します。探す順は、自分が制約で受け取った実装（その親のトレイトの実装を含む）、`impl` の順です。
- 実装は値なので、3つの実行方式は変わりません。メソッドの呼び出しは、実装のフィールドを通した間接の呼び出しになります。
- 同じトレイトと型の組に `impl` が二つあると、名前の順で最初のものを使います（重なりは検査しません）。
- メソッドは自分のジェネリック引数を持てません。トレイトの型引数で書きます。

`std` の `Eq` と `Show` は演算子と組み込み関数に結びついています。`Eq` の実装がある型では `==` と `!=` がその実装の `eq` を、`Show` の実装がある型では `to_str` と `put` がその実装の `show` を使います。実装が無い型では、組み込みの比較と文字列化のままです。`int` `str` `bool` は、細分化型の検査が `==` を直接扱うので、いつも組み込みです。

### 3.8 テスト

```
use std -> *

test '足し算' {
    assert_eq(add(1, 2), 3)
    assert(add(0, 0) == 0)
}

test '引き算' {
    let base = 10
    describe '正の数' {
        it '1 を引く' { assert_eq(base - 1, 9) }
        it '10 を引く' { assert_eq(base - 10, 0) }
    }
}
```

- `test '名前' { 本体 }` は、引数の無い関数として型検査します。`hibana test` でだけ実行し、ほかの実行方式では呼びません。`test` は宣言の先頭で文字列が続く時だけ特別で、変数名にも使えます。
- 本体で実行時エラーが起きると、そのテストは失敗です。std の `assert(ok)` と `assert_eq(got, want)`（`Eq` と `Show` の実装が要る）は、組み込みの `panic` で実行時エラーにします。失敗の位置は、ライブラリの中で起きたエラーでも、テストのファイルの中の呼び出しにします。
- `main` の無いファイルには、空の `main` を足して検査します。
- 本体の中の `describe '名前' { ... }` はテストをまとめ、`it '名前' { ... }` は1件のテストになります。`it` があると、`it` ごとに1件として実行し、名前は `test` から順に ` / ` でつなぎます（`引き算 / 正の数 / 1 を引く`）。`it` より前に書いた文（外側の `test` と `describe` の中のもの）は、その `it` の本体の前に毎回実行します。`it` の無い `describe`、最後の `it` と `describe` の後の文はエラーです。`describe` と `it` は、後に文字列と `{` が続く時だけ特別で、変数名にも使えます。

### 3.9 型に属する関数とメソッド

```
struct Param { value: str }

fn Param::get(p: Param) -> str { p.value }     // 型 Param に属する関数
fn str::twice(s: str) -> str { s + s }         // 組み込みの型にも書ける

fn main() {
    let id = Param { value: 'u7' }
    put(Param::get(id))       // u7
    put(id.get().twice())     // u7u7。x.f(a) は、x の型 T の T::f(x, a) を呼ぶ
}
```

- `型の名前::名前` は1つの名前です。関数の宣言・呼び出し・`use ... -> { 名前 }` のどこでも書けます。
- `x.f(a)` は、`x` の型を求めてから `T::f(x, a)` の呼び出しに書き換えます。`T::f` が無ければエラーです。`str`・`int`・`Vec` などの組み込みの型のメソッドは、どのモジュールで宣言しても使えます。
- ponytail: 名前は内部で `T__f` になり、エラーにもその名前で出ます。同じ組み込みの型に同じ名前のメソッドを複数のモジュールが宣言すると、名前の順で最初のものを使います。

## 4. 式

| 式 | 例 |
|---|---|
| リテラル | `1` `'s'` `` `n={n}` `` `true` `[1, 2, 3]` |
| 変数 | `let x = 1`、`let mut i: int = 0`、`i = i + 1` |
| 演算 | `+ - * / % **`（`+` は文字列の連結にも使う。`**` はべき乗）、`< <= > >= == !=`、`&& \|\| !` |
| 呼び出し | `f(x)`、`f<int>(x)`、`adder(10)(5)` |
| 条件 | `if c { a } else { b }`（`else` が無ければ値は捨てる。片方の分岐だけで型が決まらない `None` や `[]` は、もう片方の型になる） |
| 繰り返し | `while c { }`、`for x in v { }`（`v` は `Vec`）、`for i in a..b { }` |
| 場合分け | `match o { None => 0, Some(x) => x }`（腕は改行か `,` で区切る。`_` は残りすべて） |
| 無名関数 | `fn(x: int) -> int { x + 1 }`、期待する型があれば `fn(x) { x + 1 }` |
| 配列 | `v[i]`、`len(v)` |
| 構造体 | `Point { x: 1, y: 2 }`、`p.x` |
| ブロック | `{ let a = 1; a + 1 }`（最後の式が値） |

- `match` は網羅していなければエラーです。
- `**` は単項の `-` `!` より強く結合し、右から結合します（`-x ** 2` は `-(x ** 2)`、`2 ** 3 ** 2` は `2 ** (3 ** 2)`）。負の指数は実行時エラー（負の指数）、結果が `int` の範囲を超えると実行時エラー（オーバーフロー）です。
- 無名関数は作った時の変数の値を写し取ります。後から代入しても関数の中には届きません。

## 5. 型

| 型 | 意味 |
|---|---|
| `int` `bool` `str` `unit` | 64ビット整数・真偽値・文字列・値の無いこと |
| `Vec<T>` | 配列。書き換えられないので、`Vec<int[x -> x > 0]>` は `Vec<int>` として使える |
| `Ref<T>` | 書き換えられる参照。`new_ref(x)` で作り、`read(r)` で読み、`write(r, x)` で書く。無名関数や handle の腕が写し取っても同じ値を指す（`mut` 変数は写し取った時の値になる） |
| `(x: A, B) -> C` | 関数型。引数名は省ける。細分化型の条件では前の引数の値を参照できる |
| `T[x -> 条件]` | 細分化型。条件を満たす `T` の値だけを許す |
| `Type` | 型引数に指定する種類。`<A: Type>` は `<A>` と同じ |
| `syn`、`syn<種類>` | コード片。マクロ本体の中でだけ使う（8 節） |

## 7. 検査

### 7.1 細分化型の証明

細分化型の条件と配列の添字の範囲（`0 <= i && i < len(v)`）は、コンパイル時に証明します。

1. 値が分かれば計算して確かめます。
2. 分からなければ、その位置で成り立つ事実と照合します。事実は、細分化型を持つ変数・引数・構造体のフィールドの条件、`if` / `while` の条件（`else` 側では否定）、`for i in a..b` の `a <= i < b`、`a && b` の `b` での `a`（`||` では否定）、ループを抜けた後の条件の否定から集めます。
3. 照合では、字面の一致と `a <= b + k` の形の弱め（例: `0 <= i` から `0 <= i + 1`、`2 <= n` から `0 <= n - 1`）を試します。両辺の定数は `k` に寄せて比べます。次に、`x == 定数` の事実で目標の `x` を置き換えて計算します（例: `len(parts) == 2` から `0 < len(parts)`）。
4. それでも示せなければ Z3 に問い合わせます。`HIBANA_NO_Z3` を設定すると Z3 を使いません。例のうち Z3 が要るのは、`smt.hb` と掛け算の条件を持つ `struct.hb` だけです（比較・`&&` `||` `!`・`+ - *` を Z3 の項に訳し、訳せない項は記号にします）。

代入された変数についての事実は捨てます（型の条件は残します）。`while` / `for` の中で代入される変数についての、ループに入る前の事実も捨てます。

## 8. マクロ

### 8.1 構文の宣言

```
syntax Route = annotations:Annotation* method:Ident path:String body:Block
macro routes ('routes' '{' rs:Route* '}') -> Expr { ...本体... }
```

パターンの書き方:

| 書き方 | 意味 |
|---|---|
| `'lit'` | リテラル（1個のトークンでなければならない。`'Response::json'` は1つの名前として照合する） |
| `Ident Int Float String Template Expr Block Stmt Type Text` | 組み込みの項。`Template` は `` `...` `` の文字列で、`str_of` はソースのままの中身（`{ }` を含む）を返す。`Text` は、後に続きうるトークンの手前までの生の文字列 |
| `Name` | 他の `syntax` 規則 |
| `x:P` | 束縛。同じ連接の中で同じ名前を2回使うとエラー |
| `P*` `P+` `P?` `P % sep` | 0回以上・1回以上・省略可能・区切り付きの並び |
| `~P` | 直前のトークンと空白なしで続く |
| `A \| B` | 順序付きの選択（先に一致した方を採る） |
| `( P )` | グループ |

- マクロは、式の位置（`-> Expr`、省略時）か文の位置（`-> Stmt`）で、組み込みの構文より先に試します。宣言の位置（`-> Decl`）のマクロは、`fn` や `type` などで始まらない宣言の位置で試します（8.4 節）。一致しなければ組み込みの構文として読みます。複数のマクロが一致すると「Ambiguous macro match」エラーです。
- 左再帰（`syntax E = E '+' Int`）と、`Expr` / `Stmt` で始まるマクロは禁止です。
- `syntax` 宣言の末尾の `;` は省けます。

### 8.2 本体

マクロ本体はコンパイル時に実行する普通の hibana のコードで、最後の値（`syn` 型）が展開結果になります。本体が空なら照合結果だけを残し、型検査で「展開コードが無い」エラーになります。

```
macro pow ('pow' '(' x:Expr ',' n:Int ')') {
    let mut acc = quote { 1 }
    let mut i = 0
    while i < int_of(n) {
        acc = quote { $acc * $x }
        i = i + 1
    }
    acc
}
```

- `quote { ... }` はコード片を作ります。中では `$x`（変数の値）、`$x.f`（フィールド）、`${式}`（式の値）、`$( ... ) 区切り? *`（配列を繰り返す）が使えます。差し込める値は `syn`・`int`・`str`・`bool` です。
- パターンで束縛した式は、括弧で囲んで差し込みます（`-sum(1 + 2)` は `-(1 + 2)`）。`quote` で作った断片は囲みません。
- 束縛の型は、パターンから決まる `syn<種類>` です（`p:Pair` は `syn<Pair>`、`ps:(Pair % ',')` は `syn<[Pair]>`、`x:Expr` は `syn<Expr>`）。存在しないフィールドの参照や、配列でない値への `at` は、マクロ定義の時点で型エラーになります。
- 組み込み関数: `len(x)`、`at(xs, i)`、`is_null(x)`、`int_of(x)`、`str_of(x)`、`ident(s)`（文字列から識別子を作る）、`error(msg)`（コンパイルエラーにする）。文字列の関数（9 節）も使えます。マクロ本体の `put` は標準エラーに出ます。
- 本体から普通の `fn` は呼べません。マクロは関数の構文解析より前に実行するためです。

### 8.3 衛生性

`quote` に直接書いた識別子は、展開ごとに固有の名前（`tmp#3`）に付け替えます。展開したコードの変数が、呼び出し側の変数と衝突しません。関数名・型名・構築子名・組み込み関数・パターンのリテラル・`.` の後のフィールド名・`Name { x: 式 }` の `x`（構造体のフィールド名）は付け替えません。ただし、同じ `quote` の中で `let`・`for` の変数、`fn` の引数、`match` と `handle` の腕の束縛になる名前は、関数名や組み込みの関数と同じでも付け替えます（`let len = 1` の `len` など）。その名前を同じ `quote` の中で大域の関数として呼ぶと、その呼び出しも付け替えられるので、避けてください。`$x` で差し込んだ呼び出し側のトークンと、`ident(s)` で作った識別子も付け替えないので、呼び出し側から見える名前を導入できます。

### 8.4 宣言のマクロ

`-> Decl` のマクロは、トップレベルの宣言の位置で呼び、展開結果を宣言の並びとして読みます。`syntax` と `macro` の宣言はほかの宣言より先に読むので、宣言のマクロは後で（`use` したライブラリの中で）宣言したものでも使えます。

```
syntax DeriveType =
    | 'struct' name:Ident params:DeriveParams? '{' fields:DeriveField* '}'
    | 'enum' name:Ident params:DeriveParams? '{' ctors:DeriveCtor* '}'
macro derive ('derive' '(' traits:(Ident % ',') ')' t:DeriveType) -> Decl {
    ...
    quote { $t impl Eq<${t.name}> { fn eq(a: ${t.name}, b: ${t.name}) -> bool { ... } } }
}
```

- 展開は、すべての宣言の名前を集めた後にします（衛生性で、後で宣言した名前も大域の名前として扱うため）。展開した宣言を含めて、ソースを最初から読み直します。展開結果がまた宣言のマクロを含めば、16回まで繰り返します。
- 型の宣言を受け取る組み込みの項はありません。型の宣言の形は `syntax` で書きます（std の `DeriveType` など）。
- `quote` の中の `_`（`match` のどれにも合わない腕）は付け替えません。

std の `derive` は、型の宣言と、その型の `Eq`・`Show` の `impl` を作ります（例は `tests/programs/macros/derive.hb`）。

```
derive(Eq, Show)
enum Shape { Circle(r: int), Rect(w: int, h: int), Dot }
```

- `Eq` は、同じ構築子でフィールドがすべて `==` なら等しいとします。`Show` は組み込みの文字列化と同じ形（`Point { x: 1, y: 2 }`、`Rect(3, 4)`）で、フィールドは `to_str` で文字列にします。どちらも、フィールドの型に実装があればその実装を使います。
- 型引数を持つ型では `impl<A, B> Eq<Pair<A, B>>` を作ります。型引数に制約は付けないので、型引数の型の値は組み込みの比較と文字列化になります。
- `struct Name(型)` の構造体と `Ord` の derive はできません。

## 9. 組み込み関数

| 関数 | 型 |
|---|---|
| `put(x)` | 任意の値を表示する |
| `len(v)` | `Vec<T>` か `str` の長さ。`mut` の変数は、初期値と `push`・`set_at` の代入から長さを追う（ループと分岐の後は分からなくなる） |
| `to_str(x)` | 任意の値を文字列にする |
| `split(s, sep)` | `(str, str) -> Vec<str>` |
| `starts_with(s, p)` | `(str, str) -> bool` |
| `slice(s, i, j)` | `(str, int, int) -> str`。範囲外は実行時エラー |
| `chars(s)` `code(c)` | `str -> Vec<str>`（1 文字ずつ）、`str -> int`（先頭の文字の番号。空なら -1） |
| `panic(msg)` | `str -> never`。実行時エラーにする |
| `now_ms()` | `() -> int`。1970 年 1 月 1 日（UTC）からのミリ秒 |
| `new_ref(x)` `read(r)` `write(r, x)` | `T -> Ref<T>`、`Ref<T> -> T`、`(Ref<T>, T) -> unit` |
| `push(v, x)` | `(Vec<T>, T) -> Vec<T>`。`v` の後ろに `x` を足した新しい `Vec`（`v` は変わらない）。`xs = push(xs, x)` の形で、`xs` の配列を他から指していなければ、写さずにその場で足す（平均 O(1)） |
| `set_at(v, i, x)` | `(Vec<T>, int, T) -> Vec<T>`。`v` の `i` 番目を `x` にした新しい `Vec`。`i` が範囲内であることを添字と同じく証明する。`xs = set_at(xs, i, x)` の形は `push` と同じく、その場で書き換える（O(1)） |
| `all(fs)` | `(Vec<() -> T ! Async>) -> Vec<T> ! Async`（3.5 節） |
| `resume(x)` | `handle` の腕の中で、続きを再開する（3.5 節） |

次の関数はインタプリタとマクロで使います。生成するプログラムから到達するとエラーになります。ファイルの経路は作業ディレクトリからの相対経路です。

| 関数 | 型と動作 |
|---|---|
| `args()` | `() -> Vec<str>`。`--` の後の引数 |
| `read_text(path)` / `read_bytes(path)` | `str -> str` / `str -> Vec<int>`。UTF-8 とバイト列の読み込み |
| `write_text(path, text)` / `write_bytes(path, bytes)` | `(str, str) -> unit` / `(str, Vec<int>) -> unit`。親ディレクトリを作って書き込む。バイトは 0〜255 |
| `compile_file(path, target, exports)` | `(str, str, Vec<str>) -> Vec<int>`。ファイルのディレクトリを基準に `use` を解決してコンパイル |
| `inspect_file_exports(path, exports)` | `(str, Vec<str>) -> str`。ファイルの公開関数の型情報を返す |
| `compile(source, target, exports)` / `inspect_exports(source, exports)` | ソース文字列をコンパイルし、または型情報を返す |
| `utf8(text)` / `js_glue(metadata)` | 文字列をバイト列に変換 / 型情報から JavaScript 接続コードを生成 |

## 10. ライブラリ

### std

`use std -> *` で、修飾なしに使うのが普通です（`lib/std.hb`、例は `tests/programs/stdlib/std.hb`）。

| 名前 | 内容 |
|---|---|
| `Option<A>` = `None` \| `Some(value)`、`unwrap_or(o, d)` | 値が無いかもしれないこと |
| `Result<A, E>` = `Ok(value)` \| `Err(error)` | 成功か失敗 |
| `Eq<A>`（`eq`）、`Ord<A>: Eq<A>`（`lt`）、`Show<A>`（`show`） | トレイト（3.7 節）。`int` `str` `bool` の `Eq` と `Show`、`int` の `Ord`、`Vec<A>` の `Eq` と `Show` の実装がある（`put` と `to_str` での `Vec` の表示は組み込みのまま） |
| `max(a, b)` `min(a, b)` `contains(xs, x)` | `Ord` と `Eq` を使う関数 |
| `assert(ok)` `assert_eq(got, want)` | テスト（3.8 節）。偽か等しくなければ実行時エラー |
| `derive(Eq, Show) struct ...`（`enum` も） | 型の `Eq`・`Show` の `impl` を作る宣言のマクロ（8.4 節） |
| `map(xs, f)` `filter(xs, f)` `fold(xs, init, f)` | `Vec` の高階関数。`f` の効果をそのまま起こす（効果の多相） |
| `concat(a, b)` `reverse(xs)` `take(xs, n)` `drop(xs, n)` `last(xs)` `range(a, b)` `sum(xs)` | `Vec` の操作。`last` は `Option` を返す。`range` は `a` 以上 `b` 未満の `int` の `Vec` |
| `some(xs, f)` `every(xs, f)` `find(xs, f)` | 一つでも・すべてが条件を満たすか、最初の要素（`Option`）。`f` の効果をそのまま起こす。`all` は並行実行の組み込みなので `every` と呼ぶ |
| `join(xs, sep)` `repeat(s, n)` `trim(s)` `replace(s, from, to)` `lines(s)` `ends_with(s, w)` `includes(s, w)` | 文字列の操作 |
| `Map<V>`、`empty()` `get(m, k)` `set(m, k, v)` `keys(m)` | 文字列のキーから値への表。`set` は新しい `Map` を返す。線形探索 |
| `Json` = `JNull` \| `JBool` \| `JInt` \| `JStr` \| `JArr(Vec<Json>)` \| `JObj(Vec<Field>)`、`Field { key, value }` | JSON の値 |
| `stringify(j)` `quote_str(s)` | JSON の文字列にする |
| `parse_json(s)`、`field(j, key)` `str_or(o, d)` `int_or(o, d)` | JSON を読む（`Option<Json>`、読めなければ `None`）。フィールドと値を取り出す |

- `empty()` の型引数は引数から決まらないので、`let e: Map<int> = empty()` のように期待する型を書きます。
- JSON の数は整数だけを読み、`\uXXXX` のエスケープは読めません。

### web

`use web` で、Web アプリのための型・関数・マクロを使えます（`lib/web.hb`、例は `tests/programs/web/web.hb`）。

| 名前 | 内容 |
|---|---|
| `Request` | `{ method: str, path: str, query: str, headers: Vec<Header>, body: str }`。`Header` は `{ name, value }` で、名前は小文字。`request(method, path)` で試しの要求を作る |
| `header(req, name)` `query(req, name)` | ヘッダーとクエリ（`?a=1&b=2`）の値。無ければ `''` |
| `Response` | `{ status: int, content_type: str, body: str, headers: Vec<Header> }`。直接構築するときは `headers` も必要 |
| `ok(body)` `html(body)` `json(body)` `status(code, body)` `not_found()` | 空の `headers` を持つ応答を作る（200 の text/plain・text/html・application/json、任意の番号の text/plain、404） |
| `with_header(res, name, value)` | 元の応答を変えず、ヘッダーを末尾に追加した応答を返す。同じ名前も残すので、複数の `Set-Cookie` を返せる |
| `redirect(location)` | 303 See Other、`Location` ヘッダー、空の本文を持つ応答を作る。POST の保存後に GET へ遷移するために使う |
| `routes { メソッド 'パス' { 本体 } ... }` | `(Request) -> Response` の関数を作る。パスの `{名前}` はその名前の変数になる。どれにも合わなければ `not_found()`。ルートの本体では、要求を `req` として使える。`routes ! Async, Need<Db> { ... }` と書くと、ルートの本体でその効果を起こせる（`(Request) -> Response ! Async, Need<Db>` の関数になる） |
| `serve(port, app)` | Node.js の HTTP サーバーとして `app: (Request) -> Response ! Async` を動かす（`Async`）。`port` が 0 なら空いている番号を使う。起動すると URL を表示する |
| `worker(app)` | Cloudflare Workers の `fetch` として `app` を動かす。`use web` したプログラムの JS の出力は `export default { fetch }` を持つ ES モジュールになる |
| `@f(引数) -> x` | `routes` のルートの前に書く注釈。本体の前で `let x = f(引数)` する |
| JSX | `<div class={c}>Hello {name}</div>` を HTML の文字列にする。閉じタグが合わなければコンパイルエラー |

```
use web

fn main() {
    let app = routes {
        GET '/' { web.html(<h1>Welcome</h1>) }
        GET '/user/{id}' { web.ok('user ' + id) }
    }
    let res = app(web.request('GET', '/user/42'))
    put(res.body)    // user 42
}
```

サーバーの例は `tests/programs/web/server.hb` です。データの置き場（`Ref` を持つ構造体）を `Need<Db>` の効果で受け取り、`main` の `handle` で本物の置き場を渡しています（テストでは置き場を差し替えられる）。

```
let api = routes ! Need<Db> {
    GET '/users/{name}' { if exists(name) { web.ok('found ' + name) } else { web.not_found() } }
}
let app = fn(req: web.Request) -> web.Response { handle api(req) { need() => resume(db) } }
web.serve(8080, app)    // Rust API の hibana::to_js で生成した JS を Node.js で実行
```

`serve` と `worker` は JS ターゲットでだけ動きます（インタプリタでは、呼ぶと実行時エラー）。

応答の `content_type` は既定の `Content-Type` です。`headers` に `Content-Type` を追加すれば上書きでき、名前の大文字・小文字は区別しません（複数あれば最後の値）。それ以外の同名ヘッダーは追加し、`Set-Cookie` は結合せず別々に送ります。不正なヘッダー名や改行を含む値など、送信先の HTTP API が受け付けない応答は 500 にします。`with_header` 自体は値を検査せず、HTTP へ送るときに検査します。

```
// 記事を保存した後の応答。Cookie の署名やセッション検証は別途必要
let res = web.with_header(
    web.redirect('/posts/42'),
    'Set-Cookie',
    'notice=saved; HttpOnly; SameSite=Lax; Path=/'
)
```

`redirect` は遷移先を検証しません。外部入力をそのまま渡すと意図しないサイトへ誘導できるため、アプリ側で許可したパスを使います。

`routes` の展開は、ルートの本体から見えない変数（`req` `parts`）を衛生性で隠しています。パスの区切りの位置 `parts[k]` は、条件の `len(parts) == N &&` から範囲を証明しています。

## 11. 実行方式

目標は Web とネイティブの両方です。型検査を通ったプログラムから型を消し、出力先ごとのバックエンドでコードを作ります。型と証明は実行に影響しないので、どのバックエンドも普通の値と関数だけを扱います。

### 11.1 プロジェクトのスクリプト

`config.hibana.toml` の `[scripts]` に、実行するシェルコマンドを文字列で定義します。パッケージと依存を記述する `hibana.toml` とは別のファイルです。

```toml
[scripts]
build = "hibana build.hb"
dev = "hibana dev.hb"
```

`hibana build` と `hibana dev` は同名のスクリプトを呼びます。任意名のスクリプトを呼び出す CLI はありません。現在のディレクトリから親へ探し、最初に見つかった `config.hibana.toml` だけを読みます。そのファイルにスクリプトが無くても、さらに親の設定には戻りません。設定ファイルのあるディレクトリを作業ディレクトリとして、`sh -c` で実行します。標準入出力と環境変数を継承し、実行中の hibana のディレクトリを `PATH` に追加します。

追加引数は `hibana dev -- --port 3000` や `hibana dev -- "src with spaces"` のように `--` の後へ書きます。引数がある場合、コマンド文字列の末尾に `"$@"` を付け、引数の値自体はシェルの位置引数として渡します。空白やシェルのメタ文字を含む値も一つの引数として保ちます。複数のコマンドをつないだスクリプトでは、末尾のコマンドへの追加になります。

スクリプトの終了コードを CLI の終了コードにします。Unix でシェルがシグナルにより終了した場合は `128 + シグナル番号` にします。設定が無い・不正な TOML・`[scripts]` がテーブルでない・値が文字列でない・名前が未定義の場合は診断を出し、終了コード 1 にします。CLI の引数の書式が不正なら終了コード 2 です。

`[dependencies]` に記録した外部ツールのコマンドを、スクリプト実行時の `PATH` の先頭へ追加します。それ以外は継承した `PATH` から解決します。異なる依存が同じコマンド名を公開した場合は診断を出します。その場合はスクリプトに `hibana dlx --bin コマンド npm:名前` のように識別子を書いて区別してください。`hibana new` はこの設定ファイルを作りません。

外部ツールは取得元を含む `npm:名前[@版]` または `cargo:名前[@版]` で指定します。npm の scoped package は `npm:@scope/名前[@版]` です。パッケージの識別子と、そのパッケージが公開するコマンド名は別です。

```sh
hibana add npm:vite@6.1.0      # プロジェクトの設定に追加して取得
hibana add -g cargo:ripgrep   # ユーザー共通の bin にコマンドを公開
hibana dlx npm:vite --help # 設定には追加せず取得・実行
hibana dlx --bin コマンド npm:パッケージ 引数
```

```toml
[dependencies]
"npm:vite" = "6.1.0"
"cargo:ripgrep" = "*"

[scripts]
build = "vite build"
```

`add` は最も近い `config.hibana.toml` を更新し、無ければ現在のディレクトリに作ります。値は版の文字列（省略時 `"*"`）です。コメントや書式は保持しません。Git ベースの言語パッケージは引き続き `hibana.toml` で管理します。

npm はシステムの `npm install`、Cargo は `cargo install` を使います。npm ツールの実行には Node.js、Cargo の初回取得にはビルド環境が必要です。取得・ビルド時にはパッケージ由来のスクリプトも実行されるため、信頼できるパッケージだけを指定してください。取得結果は `${XDG_DATA_HOME:-$HOME/.local/share}/hibana/` 内の `tools/v3/`（プロジェクト npm）、`tools/v4/`（プロジェクト Cargo）または `tools/v1/`（dlx・global）に保存し、言語パッケージと同じファイル単位 CAS を使います。Cargo の一時ビルド成果物は公開前に除きますが、npm/Cargo 自身のキャッシュはそれぞれ残ります。

`add -g` のコマンド配置先は同じデータディレクトリの `bin/` です。利用するにはそのパスをシェルの `PATH` に追加してください。別パッケージや手動配置の同名ファイルは上書きしません。`dlx` はパッケージ内の唯一のコマンド、またはパッケージ名と同名のコマンドを選び、それ以外は `--bin` を要求します。作業ディレクトリと標準入出力を継承し、終了コードも返します。

`dlx` の Hibana 側オプションは識別子より前に指定します。識別子の後ろは、先頭にある任意の `--` 一つだけを除き、すべて実行先への引数です。たとえば `hibana dlx cargo:bottom --version` は `--version` を bottom へ渡します。後ろの `--bin` や `--rev` も実行先への引数になり、Hibana は解釈しません。

Git ベースの Hibana パッケージも `dlx` で取得・実行できます。

```sh
hibana dlx hibana:https://example.com/hibana-tool.git
hibana dlx https://example.com/hibana-tool.git
hibana dlx --rev v1.0.0 hibana:https://example.com/hibana-tool.git
```

`hibana:` は省略できます。パッケージには `hibana.toml` と実行入口の `src/main.hb` が必要です。Git の取得結果は言語パッケージと同じ共通 CAS に保存し、URL と指定した版ごとに初回の解決結果を再利用します。実行時は一時ツリーを作り、依存を取得してからインタプリタで実行します。共通ストア内のファイルは変更せず、呼び出し元の設定や lock にも追加しません。作業ディレクトリ・標準入出力・終了コードは継承します。`--rev` は Hibana 専用、`--bin` は npm/Cargo 専用です。追加引数は子プロセスへ渡しますが、現状の Hibana 言語には引数を読み出す API がありません。Git 取得とプログラム実行は信頼できる取得元に限ってください。

`hibana install` は現在のディレクトリから祖先へたどり、最も近い `config.hibana.toml` または `hibana.toml` のディレクトリをプロジェクトとして選びます。そのディレクトリの npm/Cargo ツールと Hibana の Git 依存をまとめて取得します。既存の lock と設定の条件が同じ依存は固定版を使い、新規・条件を変更した依存は解決して `config.hibana.lock.toml` を更新します。設定から削除したツールの lock 記録は取り除きます。依存が無いプロジェクトでも空の lock を作れます。

`hibana install --frozen-lockfile` は lock の存在を必須とし、依存の記録不足・設定条件の変更・設定に無いツールの記録をエラーにします。保存先が空でも lock の固定版を取得しますが、成功・失敗にかかわらず lock の内容や書式は変更しません。CI ではこの形式を使えます。Hibana の Git 依存は URL と tag/branch/rev の指定も照合します。古い lock に指定条件が記録されていない場合、版指定のある依存は通常の `install` で再解決してから共有してください。

プロジェクトの `add` は設定ファイルと同じディレクトリの `config.hibana.lock.toml` に、指定した条件と取得した正確な版を記録します。スクリプト実行時もこの版を使い、別環境で保存済みツールが無い場合はその版を取得します。lock が無い依存は実行前に解決・記録します。条件を変更した依存だけ再解決し、同じ条件での `add` は既存の lock を使います。

```toml
[tools."npm:vite"]
requirement = "^6.0.0"
version = "6.1.0"
```

既存の Git 依存の記録は保持します。ただし従来形式の Hibana 依存名 `tools` はこの表と衝突するため併用できません。破損した lock は自動的に無視せずエラーにします。設定・lock のコメントや書式は保持しません。

npm ツールは各記録の `npm_lock` に npm の package-lock JSON（lockfileVersion 2/3）を文字列として保存します。推移依存の版・取得 URL・integrity を含み、別環境での再取得には `npm ci` を使います。同じ直接版でも異なる依存ツリーは別キャッシュとして扱い、ファイル内容の重複は共通 CAS で排除します。registry の HTTP(S) 配布だけを対象とし、local/git/link 依存や不正な integrity はエラーにします。ハッシュと取得内容の照合は npm が行います。旧形式で `npm_lock` が無い場合、通常の `install`・`add`・スクリプト実行で補完しますが、`--frozen-lockfile` は拒否します。Cargo ツールは `cargo_lock` に Cargo.lock の TOML を文字列として保存します。再取得時は crate ソースへ lock を復元し、`--locked` でビルドします。直接版・推移依存と registry checksum を検査し、異なる lock は別キャッシュに保存します。Cargo.lock は version 3/4 に対応し、依存の取得元は crates.io に限定します（git・path・別 registry は非対応）。`cargo_lock` の無い旧記録は通常の install・add・スクリプト実行で補完し、frozen は拒否します。Cargo のビルド環境は固定しません。npm の install scripts や OS 別依存、ビルド環境による差まで含めた同一バイナリは保証しません。`dlx` と `add -g` はプロジェクトの lock を読み書きせず、識別子・OS・CPU ごとに初回取得結果を再利用します。自動更新は行いません。`hibana update npm:名前` または `hibana update cargo:名前` は、`config.hibana.toml` に登録されたプロジェクト依存だけを対象にし、指定した版の範囲内で直接・推移依存を解決し直して lock に記録します。他の依存の lock は維持し、対象が見つからない場合や解決に失敗した場合は lock を書き換えません。引数なし・通常の依存名での `update` は Git 依存を対象にします。`dlx`・グローバル追加の個別更新は未対応です。Git 依存の `hibana remove 名前` は `hibana.toml` の直接依存を外し、残る依存を解決して lock の不要な Git 記録を取り除きます。npm/Cargo の `hibana remove npm:名前` / `hibana remove cargo:名前` は `config.hibana.toml` の依存と該当 lock だけを外します。`hibana remove -g npm:名前` / `hibana remove -g cargo:名前` は共通 `bin/` 内の該当ツール所有 shim だけを削除します。依存削除では共通 CAS のファイル・ツリーは削除しません。store の GC は未実装です。

### 11.2 コマンドとコード生成 API

ファイルを指定した実行は `hibana <ファイル>` で行います。コード生成は Rust API から利用します。

| コマンド | 出力 | 用途 |
|---|---|---|
| `hibana build [-- 引数...]` / `hibana dev [-- 引数...]` | 設定のスクリプトを実行 | ビルドと開発 |
| `hibana <ファイル> [-- 引数...]` | 構文木を直接評価して実行 | 開発中の確認。マクロもこの評価器で行う |
| `hibana fmt <ファイル>` | ファイルを整形して書き戻す | 整形。トークンの並びと改行の位置は変えず、字下げ（空白4つ）・行内の空白（`,` の後、`=` `==` `&&` `=>` `->` などの前後、`{ }` の内側）・空行（続く空行は1行、括弧の直後と閉じ括弧の前は無し）・コメントの位置を整える。設定は無い。JSX の要素と `$( )` `${ }` の中は元のまま（JSX の本文は空白も文字列になるため）。`+ - * /` の前後は元の空白のまま。行の長さで改行の位置は変えない |
| `hibana new <名前>` | `hibana.toml` と `src/main.hb` のディレクトリを作る | パッケージ（3.6 節） |
| `hibana add <git の URL> [tag]` | `hibana.toml` に依存を足して取ってくる。名前は URL の最後の部分（`hibana-` は除く） | パッケージ（3.6 節） |
| `hibana remove [-g] <依存名|npm:名前|cargo:名前>` | 言語依存・プロジェクトツール依存を設定と lock から外す。`-g` は global shim のみ削除 | パッケージ・外部ツール |
| `hibana install [--frozen-lockfile]` | 設定の全依存を取得。frozen は lock を変更せず検証 | パッケージ・外部ツール |
| `hibana fetch` | Hibana の Git 依存を取ってきて `config.hibana.lock.toml` を書く | パッケージ（3.6 節） |
| `hibana test [ファイル...]` | `test` をインタプリタで実行し、成功した数を表示する。失敗があれば終了コード 1。ファイルを省くと、`hibana.toml` のあるディレクトリの `src` と `tests` の `.hb` すべて | テスト（3.8 節） |
| `hibana lsp` | Language Server を標準入出力で動かす | エディタ（エラーの表示・整形・hover・定義へのジャンプ・補完・色付け）。VS Code と Zed の拡張機能は `editors/`（`editors/README.md`） |

WASM 出力は関数を名前で公開します。整数は符号付き 64 ビットで、JavaScript からは `BigInt` で渡します。`put` がある場合、取り込み `hibana.write(fd, 位置, 長さ)` で memory 上の 1 行（改行を含む UTF-8）を書きます。`now_ms` は取り込み `hibana.now_ms()` を呼びます。JavaScript の接続コードはこの 2 つを用意し、`put` の 1 行を `imports.hibana.print`（無ければ `console.log`）に渡します。整数の範囲外・0 による除算・`panic` などの実行時エラーでは、メッセージの文字列の位置を memory の 24 番地に書いて WASM の実行を停止します。IR が扱えない構文はコンパイル時にエラーになります。

ライブラリは `hibana::to_wasm_library(source, &exports)` に公開する関数名を渡して生成します。`main` は必要ありません。型検査はライブラリ全体に行い、WASM には公開関数と、そこから呼ぶ関数だけを含めます。未使用の関数や型が IR に未対応でも、型検査を通れば含めません。

JavaScript の接続コードは `hibana::to_js_glue(source, &exports)`、WASM と接続コードの同時生成は `hibana::to_web(source, &exports)` で行えます。`instantiate` の引数には WASM のバイト列と必要な外部関数を渡します。公開関数には安全に表せる JavaScript の整数か `BigInt` を渡し、整数の結果は `BigInt` で受け取ります。文字列やホスト関数を使う Web 向けライブラリの例は [WebAssembly](wasm.md) を参照してください。

JavaScript、IR、実行用 WASM、ネイティブの実行ファイルは、それぞれ `hibana::to_js`、`hibana::to_ir`、`hibana::to_wasm`、`hibana::to_native` で生成します。API は文字列やバイト列を返し、ファイルへの保存は呼び出し側で行います。`to_wasm` と `to_native` は `main` から辿れる関数だけを含めます。

Rust のライブラリからは、`hibana::pipeline` の各関数を組み合わせられます。`load` は `use` のファイルを読み、`parse` は構文木を作り、`check` は型検査します。`lower` は IR を作り、`link` は IR から公開関数とその呼び出し先を選びます。`emit_wasm` と `emit_js_glue` はそれぞれバイト列と文字列を返すため、書き込みや外部ツールの起動は呼び出し側で決められます。

```rust
use hibana::pipeline::{self, Kind};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = "fn twice(n: int) -> int { n + n }";
    let loaded = pipeline::load(source, Path::new("."))?;
    let parsed = pipeline::parse(&loaded)?;
    let checked = pipeline::check(parsed, Kind::Library)?;
    let ir = pipeline::lower(&checked, None)?;
    let exports = vec!["twice".to_string()];
    let linked = pipeline::link(&ir, &exports)?;
    let wasm = pipeline::emit_wasm(&linked, Some(&exports))?;
    let glue = pipeline::emit_js_glue(&linked, &exports)?;
    Ok(())
}
```

IR は WASM とネイティブの共通の中間表現です。制御フローは WASM と同じ入れ子の形（`if`・`while`・`return`）で持ちます。値はすべて、型の番号と中身の 2 つの 64 ビット整数で表します。文字列の連結・比較・表示や組み込み関数は、hibana の構文で書いたランタイム（`src/backend/runtime.hb`）にあり、同じ変換で IR にします。ランタイムは型検査を通さず、`__load` などの組み込みを IR の命令に直接変換します。効果の `handle` は、腕の値で抜ける時に中断の印を置き、呼び出しの後の検査で 1 段ずつ戻って本体を抜けます。`resume` は腕の末尾でだけ使えます。全関数が IR に対応していない場合は、`lower(&checked, Some(&exports))` に公開関数を渡すと、使う関数だけを変換できます。WASM の境界（公開関数と外部関数）は `int`・`str`・`unit` だけを渡せます。任意の外部関数、配列の受け渡し、WASM モジュール同士のリンクには未対応です。

ネイティブの出力（`hibana::to_native`）は、IR を Cranelift（Rust で書かれたコード生成器）で機械語にし、libc を使わない静的な ELF の実行ファイルを返します。リンカーは使わず、出力・終了・メモリの確保（mmap）・時刻は Linux の syscall を直接呼びます。現在は x86-64 の Linux だけに対応し、外部関数は使えません。実行時エラーでは、標準エラーにメッセージを書いて終了コード 1 で終わります。

`hibana::to_wasm` は `main` を持つ実行用 WASM を返します。実行するには WebAssembly の実行環境が必要です。

どのバックエンドも、インタプリタと同じ出力になることをテスト（`tests/programs/` と `examples/` のすべてのプログラム）で確かめています。`fib(30)`（約135万回の関数呼び出し）は、ネイティブ 0.007 秒、Node.js 0.11 秒、インタプリタ 1.8 秒でした。

## 12. 既知の制約

- `syntax` の規則名とマクロ名には名前空間がありません。別のモジュールで同じ名前の規則を定義すると衝突します。
- JavaScript との境界では `int` を number に変換するので、2^53 を超える整数は正しく渡せません。
- IR を通す WASM とネイティブの出力は、確保したメモリを回収しません（GC が無い）。
- 実行時エラーの位置は、インタプリタではエラーを起こした一番内側の式、JavaScript では最後に通った失敗しうる式（算術・`slice`・効果の操作・JavaScript の関数の呼び出し）です。JavaScript の関数の中で投げられた例外は、その呼び出しの位置になります。
- エラーの下線には、メッセージの一番内側の部分を説明に付けます。未定義の名前には綴りの近い名前（編集距離 2 以下）を、文字列が必要な所の数値には `to_str` を提案します。関数を呼んで未処理の効果が起きた時は、その関数の中で操作を呼んでいる位置にも下線を引きます（その関数が直接呼ぶ操作だけを探します）。
- Z3 には `int` を上限の無い整数として渡します。オーバーフローは実行時に検査しています。`/` と `%` は Z3 に訳しません。
- 細分化型の条件は、変数名が違うと別の型とみなします（`int[x -> x > 0]` と `int[y -> y > 0]`）。
- 実行時には型引数が消えているので、型引数を値として表示すると `?` になります。
- マクロの `Text` はトークン単位で切り出すので、字句解析できない文字（`Don't` の `'` など）を含められません。
- マクロ照合の途中で起きたコア構文のエラーは「照合失敗」として巻き戻すので、入れ子のマクロの曖昧性エラーは表に出ません。

これらの多くは、コード中に `ponytail:` の注釈で、上限と改善の方向を書いてあります。
