// == はユーザーが書いた Eq の実装を使う
use std -> *

struct P { x: int, y: int }

impl Eq for P {
    fn eq(self, other: P) -> bool { self.x == other.x }
}

fn main() {
    put(P { x: 1, y: 2 } == P { x: 1, y: 3 })
    put(P { x: 1, y: 2 } != P { x: 2, y: 2 })
}
