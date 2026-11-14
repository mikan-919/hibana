// 無名関数は作った時の変数の値を写し取る。後から代入しても関数の中には届かない
fn main() {
    let mut n = 1
    let f = fn() -> int { n }
    n = 2
    print(f())
    print(n)
}
