// 文の位置で使うマクロ
macro unless ("unless" cond:Expr body:Block) -> Stmt {
    if !$cond $body
}

macro repeat ("repeat" n:Expr body:Block) -> Stmt {
    let mut i = 0
    while i < $n { $body; i = i + 1 }
}

// 式の位置で使うマクロ。$( ... )* で配列の束縛を繰り返す
macro sum! ("sum" ~ "!" "(" xs:(Expr % ",") ")") {
    0 $(+ $xs)*
}

// syntax 規則のフィールドは $束縛.フィールド で参照する
syntax Pair = key:Ident "=" value:Expr ;

macro show ("show" "{" ps:(Pair % ",") "}") -> Stmt {
    $( print($ps.value); )*
}

fn main() {
    unless false { print(1) }
    unless true { print(2) }
    repeat 3 { print(3) }
    print(sum!(1, 2 * 3, 4))
    // 束縛した式は括弧で囲んで差し込むので -(1 + 2) になる
    print(-sum!(1 + 2))
    show { a = 10, b = 20 }
}
