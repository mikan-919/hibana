use std -> *

type Positive = number[x -> x > 0]

fn half(x: number) -> number { x / 2 }
fn positive(x: Positive) -> number { x }

fn half_first(xs: Vec<number>) -> number {
    if len(xs) > 0 { xs[0] / 2 } else { 0.0 }
}
fn choose(flag: bool) -> number { if flag { 1 } else { 2.5 } }
fn nonnegative_sum(x: number[x -> x >= 0.0], y: number[y -> y >= 0.0]) -> number[r -> r >= 0.0] { x + y }
fn nonnegative_product(x: number[x -> x >= 0.0], y: number[y -> y >= 0.0]) -> number[r -> r >= 0.0] { x * y }
fn nonnegative_division(x: number[x -> x >= 0.0], y: number[y -> y > 0.0]) -> number[r -> r >= 0.0] { x / y }
fn bounded_shift(x: number[x -> 0.0 <= x && x <= 100.0]) -> number[r -> r > x] { x + 1.0 }
fn bounded_subtract(x: number[x -> -100.0 <= x && x <= 100.0]) -> number[r -> r < x] { x - 1.0 }
fn interval_sum(x: number[x -> 1.0 <= x && x <= 2.0]) -> number[r -> 2.0 <= r && r <= 3.0] { x + 1.0 }
fn interval_product(x: number[x -> -2.0 <= x && x <= 3.0]) -> number[r -> -6.0 <= r && r <= 9.0] { x * x }

fn nonnegative_root(x: number[x -> x >= 0.0]) -> number[r -> r >= 0.0] { num_sqrt(x) }

fn main() {
    put(half_first([3, 5]))
    put([1, 2.5])
    put([1, 2] == [1.0, 2.0])
    put([1.0, 2.0] == [1, 2])
    put(choose(true))
    put(choose(false))
    put(nonnegative_sum(1.0, 2.0))
    put(nonnegative_product(2.0, 3.0))
    put(nonnegative_division(1.0, 2.0))
    put(bounded_shift(100.0))
    put(bounded_subtract(-100.0))
    put(interval_sum(1.5))
    put(interval_product(-2.0))
    let x: number = 5
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
    let joined = if len([1]) > 0 { 1 } else { 2.5 }
    put(joined / 2)
    let matched = match Some(1) { Some(_) -> 1 None -> 2.5 }
    put(matched / 2)
}
