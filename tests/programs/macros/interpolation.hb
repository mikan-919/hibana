// quote の文字列リテラルの ${式} は、文字列の全体にも途中にも、複数の場所にも埋め込める
syntax Tag = '@' ~ ident:Ident ~ '(' name:String ')'

macro Tagged(tag:Tag block:Block) {
    quote {
        put('[${tag.ident}] start: ${tag.name}')
        $block
        put('[${tag.ident}] end')
    }
}

// 構文片（syn）は綴りを、int と bool は文字列にした値を埋め込む。} を含む文字列は、${ } の中でも読める
macro pair('pair' a:Ident b:Ident) {
    quote { '${a}:${b}' }
}

macro describe('describe' n:Int) {
    quote { 'n = ${int_of(n) * 2}, ok = ${true}, braces = <${ "}" }>' }
}

fn main() {
    @section('math') { put('body') }
    put(pair x y)
    put(describe 3)
    // quote の外の文字列は、${ があっても文字どおり
    put('plain ${ text')
}
