// 入れ子のパターン（構築子・構造体・リテラル・束縛）と、let による構造体の分解
use std -> *

struct Point { x: int, y: int }
struct UserId(int)
enum Shape { Circle(center: Point, r: int), Rect(a: Point, b: Point), Empty }
struct Positive { n: int[v -> v > 0] }

// 入れ子の束縛にも、フィールドの細分化型の条件が付く（割る数が 0 でない証明）
fn per(p: Option<Positive>) -> int {
    match p {
        Some(Positive { n }) -> 100 / n
        None -> 0
    }
}

fn describe(s: Shape) -> String {
    match s {
        Shape::Circle(Point { x: 0, y: 0 }, r) -> 'origin circle r=' + r.to_string()
        Shape::Circle(Point { x, y: _ }, 1) -> 'unit circle at x=' + x.to_string()
        Shape::Circle(c, r) -> 'circle at ' + c.x.to_string() + ',' + c.y.to_string() + ' r=' + r.to_string()
        Shape::Rect(Point { x: x1, y: y1 }, Point { x: x2, y: y2 }) -> 'rect ' + ((x2 - x1) * (y2 - y1)).to_string()
        Shape::Empty -> 'empty'
    }
}

fn json_int(j: Option<Json>) -> int {
    match j {
        Some(JInt(n)) -> n
        Some(JStr('one')) -> 1
        Some(JBool(true)) -> -1
        Some(_) -> -2
        None -> -3
    }
}

fn pair(a: Option<int>, b: bool) -> String {
    match Some(a) {
        Some(Some(0)) -> 'zero'
        Some(Some(n)) -> if b { 'yes ' + n.to_string() } else { 'no ' + n.to_string() }
        Some(None) -> 'none'
        None -> 'unreachable'
    }
}

fn main() {
    put(describe(Shape::Circle(Point { x: 0, y: 0 }, 5)))
    put(describe(Shape::Circle(Point { x: 3, y: 4 }, 1)))
    put(describe(Shape::Circle(Point { x: 3, y: 4 }, 2)))
    put(describe(Shape::Rect(Point { x: 1, y: 1 }, Point { x: 4, y: 3 })))
    put(describe(Shape::Empty))
    put(json_int(parse_json('42')))
    put(json_int(parse_json('"one"')))
    put(json_int(parse_json('true')))
    put(json_int(parse_json('false')))
    put(json_int(parse_json('[')))
    put(pair(Some(0), true))
    put(pair(Some(7), true))
    put(pair(Some(7), false))
    put(pair(None, false))
    let p = Point { x: 10, y: 20 }
    let Point { x, y } = p
    put(x + y)
    let Point { x: a, y: _ } = p
    put(a)
    let UserId(id) = UserId(99)
    put(id)
    put(per(Some(Positive { n: 4 })))
    match true {
        true -> put('t')
        false -> put('f')
    }
}
