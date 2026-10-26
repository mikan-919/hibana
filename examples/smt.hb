fn Pos() -> Type { int[x -> x > 0] }
fn add(x: Pos(), y: Pos()) -> int[r -> r > 1] { x + y }
fn main() { print(add(2, 3)) }
