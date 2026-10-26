// JavaScript / TypeScript のモジュールを use する例（JS ターゲットでだけ動く: hibana js examples/js_interop.hb）
// 型は TypeScript の型宣言から読む。モジュール名はファイル名（util・greet）
use "./js/util.ts"
use "./js/greet.js"

fn main() {
    print(util.add(2, 3) * 10)
    print(util.shout("hibana"))
    print(util.applyTwice(fn(x) { x * 3 }, 2))
    let p = util.point(1, 2)
    print(p.x + p.y)
    print(len(util.words("a b c")))
    print(greet.hello("world"))
    print(greet.size(greet.Size { w: 3, h: 4 }).w)
}
