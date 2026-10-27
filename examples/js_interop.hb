// JavaScript / TypeScript のモジュールを use する例（JS ターゲットでだけ動く: hibana js examples/js_interop.hb）
// 型は TypeScript の型宣言から読む。モジュール名はファイル名（util・greet）
use "./js/util.ts"
use "./js/greet.js"
use std.*

// Promise を返す関数は Async の効果を持つ。呼ぶ関数にも ! Async を書き、await は書かない
fn twice_later(x: int) -> int ! Async {
    util.later(x) + util.later(x)
}

// all は並行に待つ。並行に動く処理の handle は互いに混ざらない
effect Tag {
    fn tag() -> str
}

fn job(name: str, n: int) -> str ! Async {
    handle {
        let a = tag()
        let x = util.later(n)
        a + to_str(x) + tag()
    } {
        tag() => resume(name)
    }
}

fn main() ! Async {
    print(util.add(2, 3) * 10)
    print(util.shout("hibana"))
    print(util.applyTwice(fn(x) { x * 3 }, 2))
    let p = util.point(1, 2)
    print(p.x + p.y)
    print(len(util.words("a b c")))
    print(greet.hello("world"))
    print(greet.size(greet.Size { w: 3, h: 4 }).w)
    print(twice_later(5))
    // Async の関数型の値も渡せる。JavaScript からは Promise を返す関数に見える
    print(util.mapLater([1, 2], fn(x) { util.later(x) + 1 }))
    print(all([fn() { job("a", 1) }, fn() { job("b", 2) }]))
    // 効果の多相な関数にも Async を渡せる（JavaScript では map の async 版を呼んで待つ）
    print(map([1, 2], fn(x) { util.later(x) * 10 }))
}
