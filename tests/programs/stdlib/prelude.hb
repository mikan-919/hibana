// Result と整数変換は use なしで使える
type Digit = int[x -> 0 <= x && x < 10]

fn parse_digit(s: String) -> Result<Digit, String> {
    let n = parse_int(s)?
    if 0 <= n && n < 10 { Ok(n) } else { Err('not a digit') }
}

fn main() {
    for s in ['5', '12', 'bad'] {
        match parse_digit(s) {
            Ok(n) -> { put(n) }
            Err(e) -> { put(e) }
        }
    }
}
