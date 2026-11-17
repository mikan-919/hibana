// 非同期。Async を宣言した関数を呼ぶと、まだ走っていない値 Async<T> になる。.await で待つと本体が走る
use std -> *

fn work(n: int) -> int with Async {
    put('work ' + n.to_string())
    n * 2
}

// .await を書いた所でだけ待つ
fn twice(n: int) -> int with Async {
    work(n).await + work(n + 1).await
}

// Async<T> の値は受け渡せる。引数は作る時に評価する
fn later(a: Async<int>) -> int with Async {
    put('later')
    a.await + 1
}

fn arg(n: int) -> int {
    put('arg ' + n.to_string())
    n
}

fn name(n: int) -> String with Async {
    'n' + n.to_string()
}

// Async のほかの要求は .await の時に使う
fn checked(n: int) -> int with Async, Fail<String> {
    if n < 0 { fail('negative') } else { n }
}

fn checked_or(n: int) -> int with Async {
    with Fail(fn(m) { exit 0 }) {
        checked(n).await
    }
}

// with の節を省いた関数は、本体から要求を推論する。再帰も扱う（ここでは Async）
fn sum_to(n: int) -> int {
    if n == 0 { 0 } else { work(n).await + sum_to(n - 1).await }
}

trait Note {
    fn note(self, m: String)
}

// 相互再帰の関数の要求も推論する（ここでは Note）
fn ping(n: int) {
    if n > 0 {
        note('ping')
        pong(n - 1)
    }
}

fn pong(n: int) {
    if n > 0 {
        note('pong')
        ping(n - 1)
    }
}

fn main() with Async {
    // .await を付けない呼び出しは本体を走らせない
    let a = work(arg(1))
    put('made')
    put(a.await)
    put(twice(5).await)
    put(later(work(10)).await)
    // 要求の多相な関数を通す呼び出しには .await を書かない
    put(map([1, 2], fn(x) { work(x).await + 1 }))
    // all は Async<T> の値を並行に走らせて合流する。結果の順は Vec の順
    put(all([name(1), name(2), name(3)]).await)
    // join は種類の違う 2 つを合流する
    let p = join(name(4), checked_or(5)).await
    put(p.first + ' ' + p.second.to_string())
    // 合流の中の失敗は、合流を待った所で起きる
    let r = with Fail(fn(m) { exit m }) {
        len(all([checked(1), checked(-1)]).await).to_string()
    }
    put(r)
    put(sum_to(2).await)
    // 期待する型の無い無名関数も、本体から要求を推論する
    let shout = fn(m: String) { note(m + '!') }
    with Note(fn(m) { put(m) }) {
        ping(3)
        shout('hey')
    }
    // spawn は Async<T> を走らせ始め、要求を持たない Task<T> を返す。then は終わった時に呼ぶ
    // JavaScript では main が戻った後に終わるので、出力が実行方式で揃うよう最後に置く
    spawn(name(6)).then(fn(s) { put('then ' + s) })
}
