// 型は値なので、関数で計算できる
type T<b: bool> = if b { int } else { str }

// 戻り値の型が引数 b の値で決まる。各分岐では b の値が分かっているものとして検査する
fn f(b: bool) -> T<b> { if b { 42 } else { "forty-two" } }

// ジェネリック引数 A は呼ぶ側で省ける
fn id<A>(x: A) -> A { x }

fn main() {
    let n: int = f(true)
    let s: str = f(false)
    print(id(n))
    print(id<T<false>>(s))
    print(T<(1 < 2)>)
}
