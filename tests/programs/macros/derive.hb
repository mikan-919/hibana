// derive: 型の宣言から Eq と Show の実装を作る宣言のマクロ（std の derive）
use std -> *

derive(Eq, Show)
struct Point { x: int, y: int }

derive(Eq, Show)
enum Shape { Circle(r: int), Rect(w: int, h: int), Dot }

derive(Show)
struct Pair<A, B> { fst: A, snd: B }

fn main() {
    let p = Point { x: 1, y: 2 }
    put(p)
    put(p == Point { x: 1, y: 2 })
    put(p != Point { x: 2, y: 1 })
    put(Rect(3, 4))
    put(Rect(3, 4) == Rect(3, 4))
    put(Circle(1) == Dot)
    put(to_str(Dot) + ' / ' + to_str(Pair { fst: 1, snd: p }))
    put(contains([Circle(2), Dot], Dot))
}
