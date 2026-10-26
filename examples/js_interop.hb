// JavaScript / TypeScript のモジュールを使う例（JS ターゲットでだけ動く: hibana js examples/js_interop.hb）
// 型は hibana 側で書く。境界で int と number、構造体とオブジェクト、関数などを変換する

extern "node:path" {
    fn basename(p: str) -> str
}

extern "./js/util.ts" {
    fn add(a: int, b: int) -> int
    fn shout(s: str) -> str
    fn applyTwice(f: (int) -> int, x: int) -> int
    fn point(x: int, y: int) -> Point
}

// モジュール名を省くとグローバル（globalThis）の関数
extern {
    fn parseInt(s: str) -> int
}

type Point = { x: int, y: int }

fn main() {
    print(basename("/usr/local/hibana.hb"))
    print(add(2, 3) * 10)
    print(shout("hibana"))
    print(applyTwice(fn(x) { x * 3 }, 2))
    let p = point(1, 2)
    print(p.x + p.y)
    print(parseInt("42") + 1)
}
