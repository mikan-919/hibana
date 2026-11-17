// JavaScript / TypeScript のモジュールを use する例。hibana::to_js で生成し、Node.js で実行する。
// 型は TypeScript の型宣言から読む。モジュール名はファイル名（util・greet）
use './js/util.ts'
use './js/greet.js'
use std -> *

// Promise を返す関数は Async の効果を持つ。呼ぶと Async<T> の値になり、.await で待つ。待つ関数にも ! Async を書く
fn twice_later(x: int) -> int with Async {
    util.later(x).await + util.later(x).await
}

// 関数型の要求にも Async を書ける（! Async と同じ）。引数の関数は、Promise を返す関数でも普通の関数でもよい
fn run_later(f: (int) -> int with Async, x: int) -> int with Async {
    f(x).await + 1
}

// all は並行に待つ。並行に動く処理の handle は互いに混ざらない
effect Tag {
    fn tag() -> String
}

fn job(name: String, n: int) -> String ! Async {
    handle {
        let a = tag()
        let x = util.later(n).await
        a + x.to_string() + tag()
    } {
        tag() -> resume(name)
    }
}

fn main() ! Async {
    put(util.add(2, 3) * 10)
    put(util.shout('hibana'))
    put(util.applyTwice(fn(x) { x * 3 }, 2))
    let p = util.point(1, 2)
    put(p.x + p.y)
    put(len(util.words('a b c')))
    put(greet.hello('world'))
    put(greet.size(greet.Size { w: 3, h: 4 }).w)
    put(twice_later(5).await)
    // Async の関数型の値も渡せる。JavaScript からは Promise を返す関数に見える
    put(util.mapLater([1, 2], fn(x) { util.later(x).await + 1 }).await)
    put(all([job('a', 1), job('b', 2)]).await)
    // 効果の多相な関数にも Async を渡せる。効果の変数を通した呼び出しには .await を書かない（JavaScript では map の async 版を呼んで待つ）
    put(map([1, 2], fn(x) { util.later(x).await * 10 }))
    // 期待する関数型の Async は、無名関数が Promise を返す関数でも使える
    put(run_later(fn(x) { util.later(x).await }, 3).await)
}
