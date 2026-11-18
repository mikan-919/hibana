// WASM とネイティブの GC が、ループで作る一時的な文字列・配列・構造体・関数の値を回収し、生きている値を残すか
struct Point { x: int, y: int }
enum List { Nil, Cons(head: int, tail: List) }
trait Log { fn log(self, m: String) -> int }

fn total(l: List) -> int {
    let mut s = 0
    let mut cur = l
    let mut go = true
    while go {
        match cur {
            Cons(h, t) -> {
                s = s + h
                cur = t
            }
            Nil -> { go = false }
        }
    }
    s
}

fn noisy(i: int) -> int with Log { Log::log('step ' + i.to_string()) }

// 0 を返す。途中で文字列・配列・構造体・関数の値・with の記録を作って捨てる
fn step(i: int, s: String) -> int {
    let p = Point { x: i, y: len(s) }
    let f = fn(k: int) -> int { k + p.x }
    let a = f(1) - i - 1 + p.y - len(s)
    a + with Log(fn(m: String) -> int { len(m + '!') - 1 }) { noisy(i) } - len('step ' + i.to_string())
}

fn main() {
    let keep = Point { x: 1, y: 2 }
    let mut list = Nil
    let mut acc = 0
    let mut names: Vec<String> = []
    for i in 0..5000 {
        let s = 'item ' + i.to_string() + ' ' + [i, i + 1, i + 2].to_string()
        acc = acc + step(i, s)
        if i % 4 == 0 { names = push(names, s) }
        if i % 1000 == 0 { list = Cons(i, list) }
    }
    put(acc)
    put(total(list))
    put(keep.x + keep.y)
    put(len(names))
    if len(names) > 3 { put(names[3]) }
}
