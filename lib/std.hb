// 標準ライブラリ。use std.* で修飾なしに使う

// ---- Option と Result ----

type Option<A> = None | Some(value: A)
type Result<A, E> = Ok(value: A) | Err(error: E)

// ---- トレイト ----

// 等しさ。実装がある型では == と != がこれを使う
trait Eq<A> {
    fn eq(a: A, b: A) -> bool
}

// 順序。lt(a, b) は a が b より前
trait Ord<A>: Eq<A> {
    fn lt(a: A, b: A) -> bool
}

// 文字列にする。実装がある型では to_str と print がこれを使う
trait Show<A> {
    fn show(x: A) -> str
}

impl Eq<int> {
    fn eq(a: int, b: int) -> bool { a == b }
}

impl Eq<str> {
    fn eq(a: str, b: str) -> bool { a == b }
}

impl Eq<bool> {
    fn eq(a: bool, b: bool) -> bool { a == b }
}

impl Ord<int> {
    fn lt(a: int, b: int) -> bool { a < b }
}

impl Show<int> {
    fn show(x: int) -> str { to_str(x) }
}

impl Show<str> {
    fn show(x: str) -> str { x }
}

impl Show<bool> {
    fn show(x: bool) -> str { to_str(x) }
}

fn max<A: Ord>(a: A, b: A) -> A {
    if lt(a, b) { b } else { a }
}

fn min<A: Ord>(a: A, b: A) -> A {
    if lt(b, a) { b } else { a }
}

fn contains<A: Eq>(xs: Vec<A>, x: A) -> bool {
    for y in xs {
        if eq(x, y) { return true }
    }
    false
}

// ---- derive ----

// derive(Eq, Show) type ... は、型の宣言と、その型の Eq・Show の impl にする
// == は各フィールドの ==、to_str は各フィールドの to_str を使う（フィールドの型に実装があればその実装）
syntax DeriveField = name:Ident ":" ty:Type
syntax DeriveCtor = name:Ident ("(" fields:(DeriveField % ",") ")")?
syntax DeriveBody = "{" fields:(DeriveField % ",") "}" | "|"? ctors:(DeriveCtor % "|")
syntax DeriveType = "type" name:Ident ("<" params:(Ident % ",") ">")? "=" body:DeriveBody

macro derive ("derive" "(" traits:(Ident % ",") ")" t:DeriveType) -> Decl {
    // 型引数があれば impl<A, B> ... Name<A, B>
    let mut ty = quote { ${t.name} }
    let mut gen = quote {}
    if !is_null(t.params) {
        ty = quote { ${t.name}<$( $t.params ),*> }
        gen = quote { <$( $t.params ),*> }
    }
    let fs = t.body.fields
    let cs = t.body.ctors
    let mut eq = quote { true }
    let mut show = quote { "" }
    if !is_null(fs) {
        // 構造体: a.x == b.x && ...、"Name { x: 1, y: 2 }"
        show = quote { ${str_of(t.name) + " {"} }
        let mut k = 0
        while k < len(fs) {
            let f = at(fs, k)
            eq = quote { $eq && a.${f.name} == b.${f.name} }
            let sep = if k == 0 { " " } else { ", " }
            show = quote { $show + ${sep + str_of(f.name) + ": "} + to_str(a.${f.name}) }
            k = k + 1
        }
        show = quote { $show + " }" }
    } else {
        // 構築子ごとの腕。"Name(1, 2)"、引数の無い構築子は "Name"
        let mut eq_arms = quote {}
        let mut show_arms = quote {}
        let mut k = 0
        while k < len(cs) {
            let c = at(cs, k)
            if is_null(c.fields) {
                eq_arms = quote { $eq_arms ${c.name} => match b { ${c.name} => true, _ => false }, }
                show_arms = quote { $show_arms ${c.name} => ${str_of(c.name)}, }
            } else {
                let mut xs = quote {}
                let mut ys = quote {}
                let mut cond = quote { true }
                let mut s = quote { ${str_of(c.name) + "("} }
                let mut j = 0
                while j < len(c.fields) {
                    let x = ident("x" + to_str(j))
                    let y = ident("y" + to_str(j))
                    if j == 0 {
                        xs = quote { $x }
                        ys = quote { $y }
                        s = quote { $s + to_str($x) }
                    } else {
                        xs = quote { $xs, $x }
                        ys = quote { $ys, $y }
                        s = quote { $s + ", " + to_str($x) }
                    }
                    cond = quote { $cond && $x == $y }
                    j = j + 1
                }
                eq_arms = quote { $eq_arms ${c.name}($xs) => match b { ${c.name}($ys) => $cond, _ => false }, }
                show_arms = quote { $show_arms ${c.name}($xs) => $s + ")", }
            }
            k = k + 1
        }
        eq = quote { match a { $eq_arms } }
        show = quote { match a { $show_arms } }
    }
    let mut out = quote { $t }
    let mut i = 0
    while i < len(traits) {
        let tr = str_of(at(traits, i))
        if tr == "Eq" {
            out = quote { $out impl $gen Eq<$ty> { fn eq(a: $ty, b: $ty) -> bool { $eq } } }
        } else if tr == "Show" {
            out = quote { $out impl $gen Show<$ty> { fn show(a: $ty) -> str { $show } } }
        } else {
            error("derive できるのは Eq と Show（" + tr + " は無い）")
        }
        i = i + 1
    }
    out
}

// ---- テスト ----

// 条件が偽なら実行時エラーにする（hibana test では、そのテストが失敗する）
fn assert(ok: bool) {
    if !ok { panic("assert が失敗した") }
}

fn assert_eq<A: Eq + Show>(got: A, want: A) {
    if got != want { panic("assert_eq が失敗した: " + to_str(got) + " と " + to_str(want)) }
}

fn unwrap_or<A>(o: Option<A>, d: A) -> A {
    match o {
        Some(x) => x
        None => d
    }
}

// ---- Vec の高階関数 ----

// f が起こす効果を E として、そのまま起こす（効果の多相）
fn map<A, B, E: Effect>(xs: Vec<A>, f: (A) -> B ! E) -> Vec<B> ! E {
    let mut out: Vec<B> = []
    for x in xs { out = push(out, f(x)) }
    out
}

fn filter<A, E: Effect>(xs: Vec<A>, f: (A) -> bool ! E) -> Vec<A> ! E {
    let mut out: Vec<A> = []
    for x in xs {
        if f(x) { out = push(out, x) }
    }
    out
}

fn fold<A, B, E: Effect>(xs: Vec<A>, init: B, f: (B, A) -> B ! E) -> B ! E {
    let mut acc = init
    for x in xs { acc = f(acc, x) }
    acc
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

// ---- JSON を読む ----

// 読んだ値と、続きの位置。ok が false なら読めなかった
type Parsed = { ok: bool, value: Json, pos: int }

fn no_parse() -> Parsed {
    Parsed { ok: false, value: JNull, pos: 0 }
}

// i 文字目（範囲外なら ""）
fn char_at(s: str, i: int) -> str {
    if 0 <= i && i < len(s) { slice(s, i, i + 1) } else { "" }
}

fn skip_ws(s: str, i: int) -> int {
    let mut j = i
    while char_at(s, j) == " " || char_at(s, j) == "\n" || char_at(s, j) == "\t" || char_at(s, j) == "\r" {
        j = j + 1
    }
    j
}

// 数字なら 0 から 9、それ以外は -1
fn digit(c: str) -> int {
    let ds = "0123456789"
    let mut r = -1
    for k in 0..10 {
        if slice(ds, k, k + 1) == c { r = k }
    }
    r
}

fn starts_at(s: str, i: int, w: str) -> bool {
    0 <= i && i + len(w) <= len(s) && slice(s, i, i + len(w)) == w
}

// ponytail: 整数だけを読む（小数・指数は読めない）
fn parse_int(s: str, i: int) -> Parsed {
    let mut j = i
    let neg = char_at(s, j) == "-"
    if neg { j = j + 1 }
    let start = j
    let mut n = 0
    while digit(char_at(s, j)) >= 0 {
        n = n * 10 + digit(char_at(s, j))
        j = j + 1
    }
    if j == start {
        no_parse()
    } else {
        Parsed { ok: true, value: JInt(if neg { 0 - n } else { n }), pos: j }
    }
}

// ponytail: \uXXXX のエスケープは読めない
fn parse_string(s: str, i: int) -> Parsed {
    let mut j = i + 1
    let mut out = ""
    let mut done = false
    let mut bad = false
    while !done && !bad {
        let c = char_at(s, j)
        if c == "" {
            bad = true
        } else if c == "\"" {
            done = true
            j = j + 1
        } else if c == "\\" {
            let e = char_at(s, j + 1)
            if e == "n" {
                out = out + "\n"
            } else if e == "t" {
                out = out + "\t"
            } else if e == "\"" || e == "\\" || e == "/" {
                out = out + e
            } else {
                bad = true
            }
            j = j + 2
        } else {
            out = out + c
            j = j + 1
        }
    }
    if bad { no_parse() } else { Parsed { ok: true, value: JStr(out), pos: j } }
}

partial fn parse_value(s: str, i0: int) -> Parsed {
    let i = skip_ws(s, i0)
    let c = char_at(s, i)
    if starts_at(s, i, "null") {
        Parsed { ok: true, value: JNull, pos: i + 4 }
    } else if starts_at(s, i, "true") {
        Parsed { ok: true, value: JBool(true), pos: i + 4 }
    } else if starts_at(s, i, "false") {
        Parsed { ok: true, value: JBool(false), pos: i + 5 }
    } else if c == "\"" {
        parse_string(s, i)
    } else if c == "[" {
        parse_array(s, i + 1)
    } else if c == "{" {
        parse_object(s, i + 1)
    } else {
        parse_int(s, i)
    }
}

// [ の後ろから読む
partial fn parse_array(s: str, i: int) -> Parsed {
    let mut items: Vec<Json> = []
    let mut j = skip_ws(s, i)
    let mut done = char_at(s, j) == "]"
    if done { j = j + 1 }
    let mut bad = false
    while !done && !bad {
        let p = parse_value(s, j)
        if !p.ok {
            bad = true
        } else {
            items = push(items, p.value)
            j = skip_ws(s, p.pos)
            let c = char_at(s, j)
            if c == "," {
                j = j + 1
            } else if c == "]" {
                done = true
                j = j + 1
            } else {
                bad = true
            }
        }
    }
    if bad { no_parse() } else { Parsed { ok: true, value: JArr(items), pos: j } }
}

// { の後ろから読む
partial fn parse_object(s: str, i: int) -> Parsed {
    let mut fields: Vec<Field> = []
    let mut j = skip_ws(s, i)
    let mut done = char_at(s, j) == "}"
    if done { j = j + 1 }
    let mut bad = false
    while !done && !bad {
        let k = parse_string(s, skip_ws(s, j))
        let colon = skip_ws(s, k.pos)
        if char_at(s, skip_ws(s, j)) != "\"" || !k.ok || char_at(s, colon) != ":" {
            bad = true
        } else {
            let v = parse_value(s, colon + 1)
            if !v.ok {
                bad = true
            } else {
                match k.value {
                    JStr(key) => { fields = push(fields, Field { key: key, value: v.value }) }
                    _ => { bad = true }
                }
                j = skip_ws(s, v.pos)
                let c = char_at(s, j)
                if c == "," {
                    j = j + 1
                } else if c == "}" {
                    done = true
                    j = j + 1
                } else {
                    bad = true
                }
            }
        }
    }
    if bad { no_parse() } else { Parsed { ok: true, value: JObj(fields), pos: j } }
}

// 文字列全体を JSON として読む。読めなければ None
fn parse_json(s: str) -> Option<Json> {
    let p = parse_value(s, 0)
    if p.ok && skip_ws(s, p.pos) == len(s) { Some(p.value) } else { None }
}

// オブジェクトのフィールド。無ければ None
fn field(j: Json, key: str) -> Option<Json> {
    let mut r: Option<Json> = None
    match j {
        JObj(fs) => {
            for f in fs {
                if f.key == key { r = Some(f.value) }
            }
        }
        _ => {}
    }
    r
}

// 文字列なら、その値。違えば d
fn str_or(j: Option<Json>, d: str) -> str {
    match j {
        Some(v) => match v {
            JStr(s) => s
            _ => d
        }
        None => d
    }
}

// 整数なら、その値。違えば d
fn int_or(j: Option<Json>, d: int) -> int {
    match j {
        Some(v) => match v {
            JInt(n) => n
            _ => d
        }
        None => d
    }
}
