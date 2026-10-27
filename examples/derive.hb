// derive: 型の宣言から Eq と Show の実装を作る宣言のマクロ（std の derive）
use std.*

derive(Eq, Show)
type Point = { x: int, y: int }

derive(Eq, Show)
type Shape = Circle(r: int) | Rect(w: int, h: int) | Dot

derive(Show)
type Pair<A, B> = { fst: A, snd: B }

fn main() {
    let p = Point { x: 1, y: 2 }
    print(p)
    print(p == Point { x: 1, y: 2 })
    print(p != Point { x: 2, y: 1 })
    print(Rect(3, 4))
    print(Rect(3, 4) == Rect(3, 4))
    print(Circle(1) == Dot)
    print(to_str(Dot) + " / " + to_str(Pair { fst: 1, snd: p }))
    print(contains([Circle(2), Dot], Dot))
}
