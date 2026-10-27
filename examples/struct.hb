// 構造体: 名前付きフィールドを持つ型
struct Point { x: int, y: int }

// 型引数を持つ構造体。フィールドの型は細分化型にもできる
struct Pair<A, B> { fst: A, snd: B }
type Pos = int[n -> n > 0]
struct Size { w: Pos, h: Pos }

// フィールドに名前の無い構造体。int とは別の型になり、x.0 で取り出す
struct UserId(int)
struct Rgb(int, int, int)

fn add(p: Point, q: Point) -> Point {
    Point { x: p.x + q.x, y: p.y + q.y }
}

fn area(s: Size) -> Pos { s.w * s.h }

fn main() {
    let p = Point { x: 1, y: 2 }
    let q = add(p, Point { y: 10, x: 20 })
    print(q)
    print(q.x + q.y)
    let pr = Pair { fst: 1, snd: "one" }
    print(pr.snd)
    print(area(Size { w: 3, h: 4 }))
    let id = UserId(42)
    let c = Rgb(255, 128, 0)
    print(id.0 + c.1)
    print(c)
}
