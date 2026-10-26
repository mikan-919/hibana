// 標準ライブラリ。use std.* で修飾なしに使う

// ---- Option と Result ----

type Option<A> = None | Some(value: A)
type Result<A, E> = Ok(value: A) | Err(error: E)

fn unwrap_or<A>(o: Option<A>, d: A) -> A {
    match o {
        Some(x) => x
        None => d
    }
}

// ---- Map（文字列のキーから値へ） ----

// ponytail: 線形探索。大きくなるならハッシュ表にする
type Entry<V> = { key: str, value: V }
type Map<V> = { entries: Vec<Entry<V>> }

fn empty<V>() -> Map<V> {
    let es: Vec<Entry<V>> = []
    Map { entries: es }
}

fn get<V>(m: Map<V>, key: str) -> Option<V> {
    let mut r: Option<V> = None
    for e in m.entries {
        if e.key == key { r = Some(e.value) }
    }
    r
}

// key の値を value にした新しい Map
fn set<V>(m: Map<V>, key: str, value: V) -> Map<V> {
    let mut es: Vec<Entry<V>> = []
    for e in m.entries {
        if e.key != key { es = push(es, e) }
    }
    Map { entries: push(es, Entry { key: key, value: value }) }
}

fn keys<V>(m: Map<V>) -> Vec<str> {
    let mut ks: Vec<str> = []
    for e in m.entries { ks = push(ks, e.key) }
    ks
}

// ---- JSON ----

type Json = JNull | JBool(b: bool) | JInt(n: int) | JStr(s: str) | JArr(items: Vec<Json>) | JObj(fields: Vec<Field>)
type Field = { key: str, value: Json }

// JSON の文字列リテラル
fn quote_str(s: str) -> str {
    let mut out = "\""
    for i in 0..len(s) {
        let c = slice(s, i, i + 1)
        if c == "\"" {
            out = out + "\\\""
        } else if c == "\\" {
            out = out + "\\\\"
        } else if c == "\n" {
            out = out + "\\n"
        } else {
            out = out + c
        }
    }
    out + "\""
}

// ponytail: 要素ごとの再帰は停止性を示せないので partial fn。Vec の要素を部分項とみなせば fn にできる
partial fn stringify(j: Json) -> str {
    match j {
        JNull => "null"
        JBool(b) => to_str(b)
        JInt(n) => to_str(n)
        JStr(s) => quote_str(s)
        JArr(items) => {
            let mut out = "["
            let mut first = true
            for x in items {
                if !first { out = out + "," }
                out = out + stringify(x)
                first = false
            }
            out + "]"
        }
        JObj(fields) => {
            let mut out = "{"
            let mut first = true
            for f in fields {
                if !first { out = out + "," }
                out = out + quote_str(f.key) + ":" + stringify(f.value)
                first = false
            }
            out + "}"
        }
    }
}
