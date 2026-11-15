// 構築子の名前の無いフィールドと、型の名前::構築子
struct Point { x: int, y: int }
enum Shape {
    Circle(Point, int)
    Rect(a: Point, b: Point)
    Empty
}
fn area(s: Shape) -> int {
    match s {
        Shape::Circle(c, r) -> 3 * r * r + c.x
        Shape::Rect(a, b)   -> (b.x - a.x) * (b.y - a.y)
        Shape::Empty        -> 0
    }
}
fn main() {
    put(area(Shape::Circle(Point { x: 1, y: 0 }, 2)))
    put(area(Shape::Rect(Point { x: 0, y: 0 }, Point { x: 2, y: 3 })))
    put(Shape::Circle(Point { x: 1, y: 0 }, 2))
}
