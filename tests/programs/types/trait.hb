// トレイト: 型ごとに実装を書ける関数の組
use std -> *

struct Point { x: int, y: int }

trait Describe {
    fn describe(self) -> String
    // 既定の実装。impl で省ける
    fn shout(self) -> String { self.describe() + '!' }
}

impl Describe for int {
    fn describe(self) -> String { 'int ' + self.to_string() }
}

impl Describe for Point {
    fn describe(self) -> String { '(' + self.x.to_string() + ', ' + self.y.to_string() + ')' }
    fn shout(self) -> String { 'POINT' }
}

// 条件付きの実装: 要素が Describe なら Vec も Describe
impl<A: Describe> Describe for Vec<A> {
    fn describe(self) -> String {
        let mut s = '['
        for x in self { s = s + x.describe() + ';' }
        s + ']'
    }
}

// 制約: 実装は呼び出す側で探して渡す
fn twice<A: Describe>(x: A) -> String { x.describe() + ' ' + x.describe() }

fn main() {
    put(1.describe())
    put(2.shout())
    put(Point { x: 1, y: 2 }.describe())
    put(Point { x: 1, y: 2 }.shout())
    put([1, 2].describe())
    put(twice([Point { x: 3, y: 4 }]))
    report()
}

// 親のトレイト: Ord の実装には Eq の実装が要る。Ord の制約から Eq も使える
impl Eq for Point {
    fn eq(self, other: Point) -> bool { self.x == other.x && self.y == other.y }
}

impl Ord for Point {
    fn lt(self, other: Point) -> bool { self.x + self.y < other.x + other.y }
}

impl Display for Point {
    fn to_string(self) -> String { 'P' + self.x.to_string() + ',' + self.y.to_string() }
}

fn same_or_max<A: Ord>(a: A, b: A) -> A {
    if a == b { a } else { max(a, b) }
}

fn report() {
    let p = Point { x: 1, y: 2 }
    let q = Point { x: 5, y: 0 }
    put(p == Point { x: 1, y: 2 })
    put(p != q)
    put(max(p, q))
    put(same_or_max(3, 7))
    put('to_string: ' + min(p, q).to_string())
    put(contains([p, q], Point { x: 5, y: 0 }))
}
