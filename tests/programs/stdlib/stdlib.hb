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

test 'str' {
    assert_eq(join(['a', 'b', 'c'], ', '), 'a, b, c')
    assert_eq(repeat('ab', 3), 'ababab')
    assert(ends_with('hibana', 'ana'))
    assert(!ends_with('a', 'ana'))
    assert(includes('hibana', 'ban'))
    assert(!includes('hibana', 'xyz'))
    assert_eq(trim('  a b \n'), 'a b')
    assert_eq(replace('a-b-c', '-', '+'), 'a+b+c')
    assert_eq(lines('x\ny'), ['x', 'y'])
}

fn main() {
    put(join(map(range(1, 4), fn(x) { to_str(x * x) }), ' '))
}

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
    assert(match parse_json('9223372036854775807') { Some(v) -> match v { JInt(n) -> n == 9223372036854775807 _ -> false } None -> false })
    assert(match parse_json('-9223372036854775808') { Some(v) -> match v { JInt(n) -> n == -9223372036854775807 - 1 _ -> false } None -> false })
    assert(match parse_json('9223372036854775808') { None -> true _ -> false })
    assert(match parse_json('-9223372036854775809') { None -> true _ -> false })
}
