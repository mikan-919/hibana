// TypeScript の number は境界で int（BigInt）と変換される
use "./js/num.ts"
use std.*

fn main() {
    print(num.add(2, 3) * 10)
    print(num.sum([1, 2, 3]))
    let xs = num.range(4)
    print(len(xs))
    if 3 < len(xs) { print(xs[3] * 2) }
}
