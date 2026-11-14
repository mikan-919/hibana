// x = push(x, e) は、x の配列を他から指していなければ写さずに足す。指していれば写す
// x = set_at(x, i, e) も同じく、写さずに i 番目を書き換える
fn main() {
    let mut xs: Vec<int> = []
    for i in 0..5 { xs = push(xs, i) }
    let ys = xs
    xs = push(xs, 99)
    let f = fn() -> int { len(xs) }
    xs = push(xs, 100)
    xs = push(xs, len(xs))
    print(ys)
    print(xs)
    print(f())
    let zs = xs
    for i in 0..len(xs) { if i < len(xs) { xs = set_at(xs, i, i * 2) } }
    print(xs)
    print(zs)
}
