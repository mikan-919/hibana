// 要求の多相。std の map・filter・fold は、渡した関数の要求をそのまま要求する
use std -> *

trait Log {
    fn log(self, msg: String)
}

fn double(x: int) -> int { x * 2 }

fn noisy(x: int) -> int with Log {
    log('item ' + x.to_string())
    x + 1
}

fn main() {
    // 要求の無い関数を渡せば、要求は無い
    put(map([1, 2, 3], double))
    // 無名関数の引数の型・戻り値の型・要求は、渡す先の関数型から推論する
    put(filter([1, 2, 3, 4], fn(x) { x % 2 == 0 }))
    put(fold([1, 2, 3], 0, fn(a, x) { a + x }))
    // Log を要求する関数を渡すと、map も Log を要求する
    let r = with Log(fn(m) { put(m) }) {
        map([1, 2], noisy)
    }
    put(r)
    // 無名関数の本体が使う Fail も、map の要求になる
    let s = with Fail(fn(m) { exit [] }) {
        map([1, 0, 2], fn(x) { if x == 0 { fail('zero') } else { x } })
    }
    put(len(s))
}
