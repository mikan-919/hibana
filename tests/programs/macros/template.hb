// .do() を使うマクロ（雛形）。本体は呼び出し位置で実行し、return はマクロの値になる
macro unless('unless' expr:Expr block:Block) {
    if ! expr.do() {
        return block.do()
    }
}

// 値を返す雛形。return しなかった時は続きの文が値になる
macro pick('pick' c:Expr a:Expr b:Expr) {
    let x = 1
    if c.do() {
        return a.do() + x
    }
    b.do() + x
}

fn early() -> int {
    unless true { return 1 }
    unless false { return 2 }
    3
}

fn main() {
    let x = 10
    put(pick true x 5)
    put(pick false x 5)
    put(early())
    let mut n = 0
    unless n > 0 { n = n + 7 }
    put(n)
    put(unless false { 'v' })
}
