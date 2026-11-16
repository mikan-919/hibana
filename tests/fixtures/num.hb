type Positive = num[x -> x > 0]

fn half(x: num) -> num { x / 2 }
fn positive(x: Positive) -> num { x }

fn main() {
    let x: num = 5
    put(half(x))
    put(1 + 0.5)
    put(0.5 + 1)
    put(-0.5)
    put(2.0 ** -2)
    put(5.0 % 2)
    put(positive(1.5))
    put(positive(2))
    put(0.5 < 1)
    put(1 == 1.0)
    put(1.0 != 2)
    put([0.5, 2.5])
    put(-0.0)
    put(0.00000001)
    put(1000000000000000000000.0)
}
