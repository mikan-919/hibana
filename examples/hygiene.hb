// 展開したコードで束縛した名前は、呼び出し側の同じ名前の変数を隠さない。
// 束縛した名前が大域の関数や組み込みの関数と同じでも、展開ごとに改名する

fn total() -> int {
    0
}

// let の変数
macro plus_one ("plus_one" "(" x:Expr ")") -> Expr {
    quote { { let total = 1
        $x + total } }
}

// 組み込みの関数 len と同じ名前の let
macro plus_len ("plus_len" "(" x:Expr ")") -> Expr {
    quote { { let len = 1
        $x + len } }
}

enum Shape {
    Circle(r: int),
    Dot,
}

// match の腕の束縛
macro add_circle ("add_circle" "(" s:Expr "," y:Expr ")") -> Expr {
    quote { match $s { Circle(total) => total + $y, Dot => $y } }
}

// 無名関数の引数
macro add_fn ("add_fn" "(" x:Expr ")") -> Expr {
    quote { { let g = fn(total: int) -> int { total + $x }
        g(1) } }
}

struct Pt {
    x: int,
    y: int,
}

// 構造体リテラルのフィールド名 x は改名しない。値の x（let で束縛した名前）は改名する
macro make_pt ("make_pt" "(" v:Expr ")") -> Expr {
    quote { { let x = $v
        Pt { x: x, y: 0 } } }
}

fn main() {
    let total = 10
    let len = 20
    print(plus_one(total))
    print(plus_len(len))
    print(add_circle(Circle(1), total))
    print(add_fn(total))
    let x = 7
    let p = make_pt(x)
    if p.x != 7 {
        panic("構造体の値の x が呼び出し側の x と違う")
    }
    if p.y != 0 {
        panic("構造体の値の y が違う")
    }
}
