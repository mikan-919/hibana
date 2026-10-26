// 標準ライブラリ（lib/std.hb）の例
use std.*
fn main() {
    let e: Map<int> = empty()
    let m = set(set(set(e, "a", 1), "b", 2), "a", 3)
    print(keys(m))
    print(unwrap_or(get(m, "a"), 0))
    print(unwrap_or(get(m, "z"), 0))
    let j = JObj([Field { key: "name", value: JStr("al\"ice") }, Field { key: "tags", value: JArr([JInt(1), JBool(true), JNull]) }])
    print(stringify(j))
    parse_demo()
}

fn parse_demo() {
    let src = " { \"name\" : \"al\\\"ice\", \"age\": -42, \"tags\": [1, true, null, []], \"o\": {} } "
    match parse_json(src) {
        Some(j) => {
            print(stringify(j))
            print(str_or(field(j, "name"), "?"))
            print(int_or(field(j, "age"), 0))
            print(int_or(field(j, "nope"), 7))
        }
        None => print("bad")
    }
    for bad in ["", "[1,", "{\"a\" 1}", "tru", "[1] x", "\"abc"] {
        match parse_json(bad) {
            Some(j) => print("unexpected " + stringify(j))
            None => print("none")
        }
    }
}
