// TypeScript の number は境界で int（BigInt）と変換される
use './js/num.ts'
use std -> *

fn main() {
    put(num.add(2, 3) * 10)
    put(num.sum([1, 2, 3]))
    let xs = num.range(4)
    put(len(xs))
    if 3 < len(xs) { put(xs[3] * 2) }
}
