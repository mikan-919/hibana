type List<A: Type> = Nil | Cons(x: A, xs: List<A>)

// 関数を受け取る関数。f の型は (A) -> B
fn map(A: Type, B: Type, f: (A) -> B, l: List<A>) -> List<B> {
    match l {
        Nil => Nil
        Cons(x, xs) => Cons(f(x), map(A, B, f, xs))
    }
}

fn sum(l: List<int>) -> int {
    match l {
        Nil => 0
        Cons(x, xs) => x + sum(xs)
    }
}

// 関数を返す関数。無名関数は作った時の n を覚えている
fn adder(n: int) -> (int) -> int { fn(x) { x + n } }
fn twice(f: (int) -> int) -> (int) -> int { fn(x) { f(f(x)) } }
fn add(a: int, b: int) -> int { a + b }

// 依存関数型: 引数 b の値で戻り値の型が決まる関数を受け取る
type T<b: bool> = if b { int } else { str }
fn call_dep(f: (b: bool) -> T<b>) -> int { f(true) }

fn main() {
    let inc = fn(x: int) -> int { x + 1 }
    print(twice(inc)(3))
    print(adder(10)(5))
    let l = Cons(1, Cons(2, Cons(3, Nil)))
    print(sum(map(int, int, adder(100), l)))
    print(call_dep(fn(b) { if b { 1 } else { "no" } }))
    let f: (int, int) -> int = add
    print(f(2, 3))
}
