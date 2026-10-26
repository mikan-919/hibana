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
}
