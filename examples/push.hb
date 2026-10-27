// x = push(x, e) は、x の配列を他から指していなければ写さずに足す。指していれば写す
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
}
