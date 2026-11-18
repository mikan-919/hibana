use std -> *

fn get_result(ok: bool) -> Result<int, String> {
    if ok { Ok(7) } else { Err('bad') }
}

fn twice(ok: bool) -> Result<int, String> {
    let n = get_result(ok)?
    Ok(n * 2)
}

// Option の ? は、None なら None で戻る
fn first_even(xs: Vec<int>) -> Option<int> {
    let x = find(xs, fn(x) { x % 2 == 0 })?
    Some(x + 100)
}

fn sum_firsts(a: Vec<int>, b: Vec<int>) -> Option<int> {
    Some(first_even(a)? + first_even(b)?)
}

fn main() {
    match twice(true) {
        Ok(n) -> { put(n) }
        Err(e) -> { put(e) }
    }
    match twice(false) {
        Ok(n) -> { put(n) }
        Err(e) -> { put(e) }
    }
    put(first_even([1, 4, 6]))
    put(first_even([1, 3]))
    put(sum_firsts([2], [4]))
    put(sum_firsts([2], [3]))
}
