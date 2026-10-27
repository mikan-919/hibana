// トレイト: 型ごとに実装を書ける関数の組
use std.*

type Point = { x: int, y: int }

trait Describe<A> {
    fn describe(x: A) -> str
    // 既定の実装。impl で省ける
    fn shout(x: A) -> str { describe(x) + "!" }
}

impl Describe<int> {
    fn describe(x: int) -> str { "int " + to_str(x) }
}

impl Describe<Point> {
    fn describe(p: Point) -> str { "(" + to_str(p.x) + ", " + to_str(p.y) + ")" }
    fn shout(p: Point) -> str { "POINT" }
}

// 条件付きの実装: 要素が Describe なら Vec も Describe
impl<A: Describe> Describe<Vec<A>> {
    fn describe(xs: Vec<A>) -> str {
        let mut s = "["
        for x in xs { s = s + describe(x) + ";" }
        s + "]"
    }
}

// 制約: 実装は呼び出す側で探して渡す
fn twice<A: Describe>(x: A) -> str { describe(x) + " " + describe(x) }

fn main() {
    print(describe(1))
    print(shout(2))
    print(describe(Point { x: 1, y: 2 }))
    print(shout(Point { x: 1, y: 2 }))
    print(describe([1, 2]))
    print(twice([Point { x: 3, y: 4 }]))
    report()
}

// 親のトレイト: Ord の実装には Eq の実装が要る。Ord の制約から Eq も使える
impl Eq<Point> {
    fn eq(a: Point, b: Point) -> bool { a.x == b.x && a.y == b.y }
}

impl Ord<Point> {
    fn lt(a: Point, b: Point) -> bool { a.x + a.y < b.x + b.y }
}

impl Show<Point> {
    fn show(p: Point) -> str { "P" + to_str(p.x) + "," + to_str(p.y) }
}

fn same_or_max<A: Ord>(a: A, b: A) -> A {
    if a == b { a } else { max(a, b) }
}

fn report() {
    let p = Point { x: 1, y: 2 }
    let q = Point { x: 5, y: 0 }
    print(p == Point { x: 1, y: 2 })
    print(p != q)
    print(max(p, q))
    print(same_or_max(3, 7))
    print("to_str: " + to_str(min(p, q)))
    print(contains([p, q], Point { x: 5, y: 0 }))
}
