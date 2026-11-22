// extern で JavaScript / TypeScript の関数を宣言する。hibana::to_js で生成し、Node.js で実行する。
use std -> *

struct Point { x: number, y: number }
struct Size { w: number, h: number }

extern './js/util.ts' {
    fn add(a: number, b: number) -> number
    fn shout(s: String) -> String
    fn applyTwice(f: (number) -> number, x: number) -> number
    fn point(x: number, y: number) -> Point
    fn words(s: String) -> Vec<String>
    fn later(x: number) -> number with Async
    fn mapLater(xs: Vec<number>, f: (number) -> number with Async) -> Vec<number> with Async
}

extern './js/greet.js' {
    fn hello(name: String) -> String
    fn size(o: Size) -> Size
}

// Promise を返す関数は Async を要求する。呼ぶと Async<T> の値になり、.await で待つ。待つ関数にも with Async を書く
fn twice_later(x: number) -> number with Async {
    later(x).await + later(x).await
}

// 関数型の要求にも Async を書ける。引数の関数は、Promise を返す関数でも普通の関数でもよい
fn run_later(f: (number) -> number with Async, x: number) -> number with Async {
    f(x).await + 1
}

// all は並行に待つ。並行に動く処理の with の提供は互いに混ざらない
trait Tag {
    fn tag(self) -> String
}

fn job(name: String, n: int) -> String with Async {
    with Tag(fn() { name }) {
        let a = tag()
        let x = later(n).await
        a + x.to_string() + tag()
    }
}

fn main() with Async {
    put(add(2, 3) * 10)
    put(shout('hibana'))
    put(applyTwice(fn(x) { x * 3 }, 2))
    let p = point(1, 2)
    put(p.x + p.y)
    put(len(words('a b c')))
    put(hello('world'))
    put(size(Size { w: 3, h: 4 }).w)
    put(twice_later(5).await)
    // Async の関数型の値も渡せる。JavaScript からは Promise を返す関数に見える
    put(mapLater([1, 2], fn(x) { later(x).await + 1 }).await)
    put(all([job('a', 1), job('b', 2)]).await)
    // 要求の多相な関数にも Async を渡せる。要求の変数を通した呼び出しには .await を書かない（JavaScript では map の async 版を呼んで待つ）
    put(map([1, 2], fn(x) { later(x).await * 10 }))
    // 期待する関数型の Async は、無名関数が Promise を返す関数でも使える
    put(run_later(fn(x) { later(x).await }, 3).await)
}
