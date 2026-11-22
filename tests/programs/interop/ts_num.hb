// TypeScript の number は有限な number として受け渡す
extern './js/num.ts' {
    fn add(a: number, b: number) -> number
    fn sum(xs: Vec<number>) -> number
    fn range(n: number) -> Vec<number>
}

fn main() {
    put(add(2, 3) * 10)
    put(sum([1, 2, 3]))
    let xs = range(4)
    put(len(xs))
    if 3 < len(xs) { put(xs[3] * 2) }
}
