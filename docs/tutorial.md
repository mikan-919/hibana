# hibana 入門

hibana は静的型付けの言語です。型検査を通ったプログラムは、インタプリタ（構文木を直接評価する実行方式）と JavaScript で同じ出力になります。この文書は、書き始めるのに必要な機能を例で順に説明します。言語の規則は `docs/spec.md` にあります。

## 1. 動かす

プロジェクトで繰り返し使うコマンドは、`config.hibana.toml` に書けます。

```toml
[scripts]
build = "hibana build.hb"
dev = "hibana src/main.hb"
```

この例では `hibana build` と `hibana dev` が設定したコマンドを呼びます。値にはシェルコマンドを書けます。引数を足す場合は `hibana dev -- 引数...` と書きます。

設定は現在のディレクトリから親へ探し、見つかったファイルのディレクトリで `sh -c` に渡します。コマンドの終了コードも引き継ぎます。パッケージ用の `hibana.toml` とは別の設定です。

`hibana add npm:vite` のように外部ツールを追加すると、`config.hibana.toml` の `[dependencies]` に `"npm:vite" = "*"` が記録され、スクリプト内では `vite build` のようにコマンド名をそのまま使えます。Cargo のツールは `hibana add cargo:名前` で追加します。npm/Cargo はシステムにインストールされているものを使い、取得時にはパッケージのスクリプトやビルドも実行します。

事前の追加なしで実行するには `hibana dlx npm:vite --help`、常用のコマンドとして追加するには `hibana add -g cargo:ripgrep` とします。グローバルの配置先 `${XDG_DATA_HOME:-$HOME/.local/share}/hibana/bin` は自分で `PATH` に追加してください。パッケージと実行コマンドの名前が違っていても扱えます。複数のコマンドから選ぶ場合は `hibana dlx --bin コマンド npm:名前 引数` とします。Hibana 側のオプションは識別子より前に書き、後ろはすべて実行先の引数として渡します。区切りの `--` は省略できます。

Hibana パッケージは `hibana dlx hibana:GitのURL`（または `hibana dlx GitのURL`）で実行します。取得元に `hibana.toml` と `src/main.hb` を用意してください。版を選ぶ場合は `hibana dlx --rev v1.0.0 hibana:GitのURL` とします。依存も共通ストアから読み込み、呼び出し元のプロジェクト設定は変更しません。渡した引数は `args()` で取得できます。

取得したツールもファイル単位の共通ストアを使います。版は `npm:vite@6.1.0` のように指定できます。プロジェクトに追加すると、設定ファイルの隣の `config.hibana.lock.toml` に取得した正確な版を記録します。設定と lock を共有すれば、別環境のスクリプト実行でもその版を取得します。npm は `npm_lock` に推移依存の版・取得 URL・整合性ハッシュも保存し、再取得は `npm ci` で行います。古い lock は通常の `hibana install` で補完してください（`--frozen-lockfile` は `npm_lock` の無い npm 記録を拒否します）。Cargo は `cargo_lock` に Cargo.lock を保存し、取得した crate ソースへ復元して `--locked` でビルドします。`cargo_lock` の無い旧記録も通常の install で補完し、frozen では拒否します。ビルド環境や生成バイナリの同一性までは保証しません。`dlx`・グローバル追加はプロジェクトの lock を使わず初回取得結果を再利用します。自動更新は行いません。プロジェクト依存の npm/Cargo ツールは `hibana update npm:名前` / `hibana update cargo:名前` で明示的に更新できます（後述）。`dlx`・グローバル追加の個別更新は未対応です。依存は `hibana remove 名前`（Git）、`hibana remove npm:名前` / `hibana remove cargo:名前`（プロジェクトの外部ツール）で設定と lock から外せます。`hibana remove -g npm:名前` / `hibana remove -g cargo:名前` は global shim だけを削除します。どの remove も共通ストアは消さず、store の GC は未実装です。

リポジトリを取得した後は `hibana install` で、設定された外部ツールと `hibana.toml` の Git 依存をまとめて取得できます。既存の lock を使い、依存の追加や条件変更があれば更新します。CI では次のように実行します。

```sh
hibana install --frozen-lockfile
```

`--frozen-lockfile` は `config.hibana.lock.toml` が無い場合や設定と食い違う場合に失敗し、lock は書き換えません。事前に通常の `hibana install` を実行して、設定と lock を一緒にコミットしてください。子ディレクトリからでも実行でき、最も近い `config.hibana.toml` または `hibana.toml` があるディレクトリだけを対象にします。

Hibana の Git 依存を更新するには `hibana update` を実行します。引数なしで推移依存を含むすべての Git 依存を更新し、`hibana update 依存名` なら指定した依存だけを更新します。`hibana.toml` の指定範囲内で Git の版を解決し直して lock に記録します。固定コミットの指定はそのままで、ブランチやタグは現在の参照先を取得します。名前指定の場合、他の依存は設定と一致する lock の版を維持します。

プロジェクトの npm/Cargo ツールは `hibana update npm:vite` や `hibana update cargo:clap` のように指定して更新できます。対象は `config.hibana.toml` の `[dependencies]` に登録したツールに限り、設定した版の範囲内で直接・推移依存を解決し直して lock に記録します。他の依存の lock は維持します。依存名が見つからない場合や取得に失敗した場合、lock は書き換えません。引数なし・通常の依存名での更新は従来どおり Git 依存だけを対象にします。

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
- 整数の `int` は 64 ビットで、範囲を超えると実行時エラーです。ほかの基本の型は `bool`、`String`（文字列）、`unit`（値が無いこと）です。

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
- `use std -> *` は標準ライブラリ（`lib/std.hb`）の名前を修飾なしで使えるようにします。`map`、`filter`、`fold`、`range`、`join`、`trim` などがあります。一覧は仕様の 10 章にあります。
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

## 4. トレイト・効果

トレイトは、型ごとに実装を書ける関数の組です。`<A: Describe>` は、`A` に `Describe` の実装があることを要求します。実装が無い型を渡すとエラーです。

効果（代数的効果）は、関数が外部に頼む操作です。`effect` で操作を宣言し、使う関数は `! 効果名` で宣言します。`handle` が操作を受け取って処理し、`resume()` で操作の呼び出しの直後から実行を再開します（再開は 1 回だけです）。

```
use std -> *

trait Describe<A> {
    fn describe(x: A) -> String
}

impl Describe<int> {
    fn describe(x: int) -> String { 'int ' + to_str(x) }
}

fn twice<A: Describe>(x: A) -> String { describe(x) + ', ' + describe(x) }

effect Log {
    fn log(msg: String)
}

fn work() ! Log {
    log('start')
    log('end')
}

fn main() {
    put(twice(3))
    handle work() {
        log(msg) -> { put('log: ' + msg); resume() }
    }
}
```

出力は `int 3, int 3`、`log: start`、`log: end` です。

- `==` は `Eq` の実装があればそれを使い、無ければ構造の比較をします。

## 5. テスト

`test '名前' { ... }` は `hibana test` が実行します。`assert(条件)` と `assert_eq(結果, 期待値)` で確かめます。ファイルを省くと、`hibana.toml` のあるディレクトリの `src` と `tests` のすべての `.hb` を実行します。

```
use std -> *

test 'join' {
    assert_eq(join(['a', 'b'], '-'), 'a-b')
}
```

## 6. 道具

- `hibana fmt <ファイル>` はファイルを整形して書き戻します。設定はありません。
- `hibana lsp` は Language Server を動かし、エディタにエラー表示・整形・hover・定義へのジャンプ・補完・色付けを渡します。VS Code と Zed の拡張機能は `editors/` にあります。
- パッケージは git のリポジトリです。`hibana new 名前` でプロジェクトを作り、`hibana add <git の URL> [tag]` で依存を足します。`hibana fetch` が依存を取ってきて、`config.hibana.lock.toml` にコミットを書きます。パッケージは `${XDG_DATA_HOME:-$HOME/.local/share}/hibana/` でプロジェクト間共有し、同じ内容のファイルは共通ストアに一度だけ保存します。詳細は仕様の 3.6 節です。
- JavaScript と TypeScript のモジュールは `use './util.ts'` で読めます。型は TypeScript の宣言から読みます。詳細は仕様の 3.4 節です。

## 7. ビルドだけで成果物を作る

次の `build.hb` はソースから JavaScript を生成します。`hibana build` または `hibana build.hb` で実行します。

```
fn main() {
    write_bytes('dist/app.mjs', compile_file('src/main.hb', 'js', []))
}
```

`compile_file` は対象ファイルを基準に `use` を解決します。`write_bytes` は親ディレクトリを作成します。引数は `hibana build -- a b` と渡し、`args()` で読めます。これらの関数はインタプリタとマクロで使い、生成したプログラムからは呼べません。

WASM と JavaScript 接続コードもスクリプトで作れます。

```
fn main() {
    let exports = ['echo']
    write_bytes('dist/app.wasm', compile_file('src/lib.hb', 'wasm', exports))
    write_text('dist/app.mjs', js_glue(inspect_file_exports('src/lib.hb', exports)))
}
```

公開する関数は `src/lib.hb` に書きます。接続コードの使い方は [WebAssembly](wasm.md) を参照してください。

## 8. 次に読むもの

- 言語の規則と既知の制約: `docs/spec.md`
- [カテゴリ別の例](../tests/programs/README.md)（効果は `tests/programs/effects/effects.hb`、トレイトは `tests/programs/types/trait.hb`、マクロは `tests/programs/macros/macros.hb`）
