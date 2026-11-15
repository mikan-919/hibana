// 文字列と整数のパターン。_ の腕が要る
fn color(s: str) -> str {
    match s {
        'red'  -> 'nice'
        'blue' -> 'cool'
        _      -> 'ok'
    }
}
fn main() {
    put(color('red'))
    put(color('blue'))
    put(color('x'))
    match 2 { 1 -> put('one'), 2 -> { put('two') }, _ -> put('many') }
    match color('red') { 'nice' -> { match 3 { 3 -> put('nested'), _ -> put('-') } }, _ -> put('?') }
}
