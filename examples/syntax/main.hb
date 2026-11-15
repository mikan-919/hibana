macro unless('unless' expr:Expr block:Block) {
    if ! expr.do() {
        return block.do()
    }
}

fn main() {
    unless false {
        put('条件が偽なので実行される')
    }
    unless true {
        put('条件が真なので実行されない')
    }
}
