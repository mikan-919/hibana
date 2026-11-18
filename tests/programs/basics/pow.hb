// べき乗。** は単項の - より強く、右から結合する
fn power(a: int, b: int) -> int {
    a ** b
}

fn main() {
    put(2 ** 10)
    put(-2 ** 2)
    put((-2) ** 3)
    put(2 ** 3 ** 2)
    put(3 * 2 ** 2)
    put(power(0, 0))
    put(power(2, 52))
    put(power(-1, 9007199254740991))
    put(power(1, 1000) + power(0, 70))
}
