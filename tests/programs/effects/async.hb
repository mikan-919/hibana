// 非同期。Async を宣言した関数を呼ぶと、まだ走っていない値 Async<T> になる。.await で待つと本体が走る
use std -> *

fn work(n: int) -> int ! Async {
    put('work ' + n.to_string())
    n * 2
}

// .await を書いた所でだけ待つ
fn twice(n: int) -> int ! Async {
    work(n).await + work(n + 1).await
}

// Async<T> の値は受け渡せる。引数は作る時に評価する
fn later(a: Async<int>) -> int ! Async {
    put('later')
    a.await + 1
}

fn arg(n: int) -> int {
    put('arg ' + n.to_string())
    n
}

fn name(n: int) -> String ! Async {
    'n' + n.to_string()
}

effect Fail {
    fn fail(msg: String) -> never
}

// Async のほかの効果は .await の時に起きる
fn checked(n: int) -> int ! Async, Fail {
    if n < 0 { fail('negative') } else { n }
}

fn checked_or(n: int) -> int ! Async {
    handle checked(n).await {
        fail(m) -> 0
    }
}

fn main() ! Async {
    // .await を付けない呼び出しは本体を走らせない
    let a = work(arg(1))
    put('made')
    put(a.await)
    put(twice(5).await)
    put(later(work(10)).await)
    // 効果の多相な関数を通す呼び出しには .await を書かない
    put(map([1, 2], fn(x) { work(x).await + 1 }))
    // all は Async<T> の値を並行に走らせて合流する。結果の順は Vec の順
    put(all([name(1), name(2), name(3)]).await)
    // join は種類の違う 2 つを合流する
    let p = join(name(4), checked_or(5)).await
    put(p.first + ' ' + p.second.to_string())
    // 合流の中の失敗は、合流を待った所で起きる
    let r = handle len(all([checked(1), checked(-1)]).await).to_string() {
        fail(m) -> m
    }
    put(r)
}
