// std の文字列と Vec の関数
use std -> *

test 'vec' {
    assert_eq(concat([1, 2], [3]), [1, 2, 3])
    assert_eq(reverse([1, 2, 3]), [3, 2, 1])
    assert_eq(reverse<int>([]), [])
    assert_eq(reverse([1]), [1])
    assert_eq(reverse(['a', 'b', 'b', 'c']), ['c', 'b', 'b', 'a'])
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

test 'try_overflow' {
    assert(match try_overflow(fn() { sum([1, 2]) }) { Ok(n) -> n == 3 Err(_) -> false })
    assert(match try_overflow(fn() { sum([9007199254740991, 1]) }) { Ok(_) -> false Err(e) -> e == 'オーバーフロー' })
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
    assert_eq(['a'].join(', '), 'a')
    assert_eq(['a', 'b', 'c', 'd', 'e'].join(''), 'abcde')
    assert_eq(trim('   '), '')
    assert_eq(replace('aaa', 'aa', 'b'), 'ba')
    assert_eq(replace('abc', '', 'x'), 'abc')
}

// Map は 1000 件のキーでも挿入と取得が合い、set は元の Map を変えない
test 'Map' {
    let e: Map<int> = empty()
    let mut m = e
    for i in 0..1000 { m = set(m, 'k' + i.to_string(), i * 2) }
    let before = m
    for i in 0..1000 { m = set(m, 'k' + i.to_string(), i * 3) }
    let mut ok = true
    for i in 0..1000 {
        if unwrap_or(get(m, 'k' + i.to_string()), -1) != i * 3 { ok = false }
        if unwrap_or(get(before, 'k' + i.to_string()), -1) != i * 2 { ok = false }
    }
    assert(ok)
    assert_eq(get(m, 'missing'), None)
    assert_eq(len(keys(m)), 1000)
    assert_eq(keys(set(set(set(e, 'a', 1), 'b', 2), 'a', 3)), ['b', 'a'])
}

// JSON の読み書きは入力の長さに比例する（長い文字列と配列）
test 'JSON' {
    let mut items: Vec<Json> = []
    for i in 0..2000 { items = push(items, JStr('s"' + i.to_string() + 'é')) }
    let text = stringify(JArr(items))
    assert_eq(match parse_json(text) { Some(j) -> stringify(j) == text None -> false }, true)
    assert(match parse_json('[1, 2') { Some(_) -> false None -> true })
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

test 'json numbers' {
    assert(match parse_json('0.1') { Some(v) -> match v { JNum(n) -> n == 0.1 _ -> false } None -> false })
    assert(match parse_json('-12.5') { Some(v) -> match v { JNum(n) -> n == -12.5 _ -> false } None -> false })
    assert(match parse_json('1e3') { Some(v) -> match v { JNum(n) -> n == 1000.0 _ -> false } None -> false })
    assert(match parse_json('1.25E+2') { Some(v) -> match v { JNum(n) -> n == 125.0 _ -> false } None -> false })
    assert(match parse_json('-25e-2') { Some(v) -> match v { JNum(n) -> n == -0.25 _ -> false } None -> false })
    assert(match parse_json('9007199254740992.0') { Some(v) -> match v { JNum(n) -> n == 9007199254740992.0 _ -> false } None -> false })
    assert(match parse_json('0e999999999999999999999') { Some(v) -> match v { JNum(n) -> n == 0.0 _ -> false } None -> false })
    assert(match parse_json('1e-999999999999999999999') { Some(v) -> match v { JNum(n) -> n == 0.0 _ -> false } None -> false })
    assert_eq(stringify(JNum(1.0)), '1.0')
    assert_eq(stringify(JNum(-0.0)), '-0.0')
    assert(match parse_json(stringify(JNum(12.0))) { Some(v) -> match v { JNum(n) -> n == 12.0 _ -> false } None -> false })
    assert(match parse_json('-0e-999999999999999999999') { Some(v) -> match v { JNum(n) -> stringify(JNum(n)) == '-0.0' _ -> false } None -> false })
    assert_eq(stringify(parse_json('[1, 2.5, {"n": -3e1}]').unwrap()), '[1,2.5,{"n":-30.0}]')
    for s in ['00', '01', '-01', '01.0', '00e1', '.5', '-.5', '1.', '1e', '1e+', '1e-', '1e1.2', '+1', 'NaN', 'Infinity', '1e309', '-1e309', '1e999999999999999999999', '[1.e2]', '{"n":2e}'] {
        assert(match parse_json(s) { None -> true _ -> false })
    }
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
