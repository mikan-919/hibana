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

fn main() ! Async {
    // .await を付けない呼び出しは本体を走らせない
    let a = work(arg(1))
    put('made')
    put(a.await)
    put(twice(5).await)
    put(later(work(10)).await)
    // 効果の多相な関数を通す呼び出しには .await を書かない
    put(map([1, 2], fn(x) { work(x).await + 1 }))
}
