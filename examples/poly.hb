// 効果の多相。std の map・filter・fold は、渡した関数の効果をそのまま起こす
use std.*

effect Log {
    fn log(msg: str)
}

effect Fail {
    fn fail(m: str) -> never
}

fn double(x: int) -> int { x * 2 }

fn noisy(x: int) -> int ! Log {
    log("item " + to_str(x))
    x + 1
}

fn main() {
    // 効果の無い関数を渡せば、効果は無い
    print(map([1, 2, 3], double))
    print(filter([1, 2, 3, 4], fn(x: int) -> bool { x % 2 == 0 }))
    print(fold([1, 2, 3], 0, fn(a: int, x: int) -> int { a + x }))
    // Log を起こす関数を渡すと、map も Log を起こす
    let r = handle map([1, 2], noisy) {
        log(m) => {
            print(m)
            resume()
        }
    }
    print(r)
    // 無名関数の効果は書いておく
    let s = handle map([1, 0, 2], fn(x: int) -> int ! Fail { if x == 0 { fail("zero") } else { x } }) {
        fail(m) => []
    }
    print(len(s))
}
