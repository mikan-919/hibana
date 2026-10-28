// == はユーザーが書いた Eq の実装を使う
use std.*

struct P { x: int, y: int }

impl Eq<P> {
    fn eq(a: P, b: P) -> bool { a.x == b.x }
}

fn main() {
    print(P { x: 1, y: 2 } == P { x: 1, y: 3 })
    print(P { x: 1, y: 2 } != P { x: 2, y: 2 })
}
