// std の文字列と Vec の関数
use std -> *

test 'vec' {
    assert_eq(concat([1, 2], [3]), [1, 2, 3])
    assert_eq(reverse([1, 2, 3]), [3, 2, 1])
    assert_eq(take([1, 2, 3], 2), [1, 2])
    assert_eq(drop([1, 2, 3], 2), [3])
    assert_eq(range(2, 5), [2, 3, 4])
    assert_eq(sum(range(0, 5)), 10)
    assert_eq(last([1, 2]), Some(2))
    assert_eq(last([1, 2]).unwrap(), 2)
    assert(some([1, 2, 3], fn(x) { x > 2 }))
    assert(every([1, 2, 3], fn(x) { x > 0 }))
    assert_eq(find([1, 2, 3], fn(x) { x > 1 }), Some(2))
}

// try は Fail の失敗を Err に、成功を Ok にする
fn half(n: int) -> int with Fail<String> {
    if n % 2 == 0 { n / 2 } else { fail('odd') }
}

test 'try' {
    assert(match try(fn() { half(4) }) { Ok(n) -> n == 2 Err(_) -> false })
    assert(match try(fn() { half(3) }) { Ok(_) -> false Err(e) -> e == 'odd' })
    assert(match try(fn() { map([2, 4], half) }) { Ok(xs) -> xs == [1, 2] Err(_) -> false })
}

test 'String' {
    assert_eq(['a', 'b', 'c'].join(', '), 'a, b, c')
    assert_eq(repeat('ab', 3), 'ababab')
    assert(ends_with('hibana', 'ana'))
    assert(!ends_with('a', 'ana'))
    assert(includes('hibana', 'ban'))
    assert(!includes('hibana', 'xyz'))
    assert_eq(trim('  a b \n'), 'a b')
    assert_eq(replace('a-b-c', '-', '+'), 'a+b+c')
    assert_eq(lines('x\ny'), ['x', 'y'])
}

fn main() with Async {
    put(map(range(1, 4), fn(x) { (x * x).to_string() }).join(' '))
    // join は種類の違う 2 つの Async の値を並行に走らせ、両方の結果を Pair で返す
    let p = join(twice_later(2), show_later(3)).await
    assert_eq(p.first, 4)
    assert_eq(p.second, '3')
}

fn twice_later(n: int) -> int with Async { n * 2 }

fn show_later(n: int) -> String with Async { n.to_string() }

test 'json string escapes' {
    assert_eq(stringify(JStr('a\t\r\nb')), '"a\\t\\r\\nb"')
    assert_eq(str_or(parse_json('"\\b\\f\\r\\t\\n\\/\\\\\\""'), 'bad'), char_from_codepoint(8) + char_from_codepoint(12) + '\r\t\n/\\"')
    assert_eq(str_or(parse_json('"\\u65e5\\u672C\\uD83C\\uDF38"'), 'bad'), '日本🌸')
    for n in 0..32 {
        let c = char_from_codepoint(n)
        assert_eq(str_or(parse_json(stringify(JStr(c))), 'bad'), c)
        assert(match parse_json('"' + c + '"') { None -> true _ -> false })
    }
    for s in ['"\\uD800"', '"\\uDC00"', '"\\uD800\\u0041"', '"\\uD800\\uDC0"', '"\\uXYZ0"', '"\\u12"', '"\\q"'] {
        assert(match parse_json(s) { None -> true _ -> false })
    }
}

test 'json int boundaries' {
    assert(match parse_json('9007199254740991') { Some(v) -> match v { JInt(n) -> n == 9007199254740991 _ -> false } None -> false })
    assert(match parse_json('-9007199254740991') { Some(v) -> match v { JInt(n) -> n == -9007199254740991 _ -> false } None -> false })
    assert(match parse_json('9007199254740992') { None -> true _ -> false })
    assert(match parse_json('-9007199254740992') { None -> true _ -> false })
}

test 'parse int' {
    for n in [0, 7, -12, 9007199254740991, -9007199254740991] {
        assert(match parse_int(n.to_string()) { Ok(parsed) -> parsed == n Err(_) -> false })
    }
    for s in ['', '-', '+1', ' 1', '1 ', '1.5', '1x', '１２', '9007199254740992', '-9007199254740992'] {
        assert(match parse_int(s) { Ok(_) -> false Err(_) -> true })
    }
}

test "number traits" {
    assert_eq(1.5, 1.5)
    assert_eq(max(1.5, 2.5), 2.5)
    assert_eq(min(1.5, 2.5), 1.5)
}
