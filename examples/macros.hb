// マクロ本体はコンパイル時に実行する普通のコード。束縛は syn 型の変数になり、
// quote { ... } で作ったコード片（syn 型）を最後に返すと、それが展開結果になる

macro unless ("unless" cond:Expr body:Block) -> Stmt {
    quote { if !$cond $body }
}

// 繰り返しを展開時に計算して、掛け算を n 個並べる
macro pow ("pow" "(" x:Expr "," n:Int ")") {
    let k = int_of(n)
    if k < 0 { error("pow の指数が負") }
    let mut acc = quote { 1 }
    let mut i = 0
    while i < k {
        acc = quote { $acc * $x }
        i = i + 1
    }
    acc
}

// 配列の束縛は len と at で辿れる
macro sum! ("sum" ~ "!" "(" xs:(Expr % ",") ")") {
    let mut acc = quote { 0 }
    let mut i = 0
    while i < len(xs) {
        acc = quote { $acc + ${at(xs, i)} }
        i = i + 1
    }
    acc
}

// syntax 規則のフィールドは .名前 で参照する。$( ... )* でも繰り返せる
syntax Pair = key:Ident "=" value:Expr

macro show ("show" "{" ps:(Pair % ",") "}") -> Stmt {
    quote { $( print(${str_of(ps.key)}); print($ps.value); )* }
}

fn main() {
    unless false { print(1) }
    unless true { print(2) }
    print(pow(3, 4))
    print(sum!(1, 2 * 3, 4))
    // 束縛した式は括弧で囲んで差し込むので -(1 + 2) になる
    print(-sum!(1 + 2))
    show { a = 10, b = 20 }
}
