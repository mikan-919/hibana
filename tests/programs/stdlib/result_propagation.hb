use std -> *

fn get_result(ok: bool) -> Result<int, String> {
    if ok { Ok(7) } else { Err('bad') }
}

fn twice(ok: bool) -> Result<int, String> {
    let n = get_result(ok)?
    Ok(n * 2)
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
}
