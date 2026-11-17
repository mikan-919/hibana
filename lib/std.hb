// 標準ライブラリ。use std -> * で修飾なしに使う

// ---- Option と Result ----

enum Option<A> { None, Some(value: A) }
enum Result<A, E> { Ok(value: A), Err(error: E) }

// 符号 - と10進数字を読み、文字列全体が整数かを検査する
fn parse_int(s: String) -> Result<int, String> {
    let parsed = parse_json_int(s, 0)
    if parsed.ok && parsed.pos == len(s) {
        match parsed.value {
            JInt(n) -> Ok(n)
            _ -> Err('整数として読めない')
        }
    } else {
        Err('整数として読めない')
    }
}

// ---- トレイト ----

// 等しさ。実装がある型では == と != がこれを使う
trait Eq {
    fn eq(self, other: Self) -> bool
}

// 順序。a.lt(b) は a が b より前
trait Ord: Eq {
    fn lt(self, other: Self) -> bool
}

// 文字列にする。実装がある型では .to_string() と put がこれを使う
trait Display {
    fn to_string(self) -> String
}

impl Eq for int {
    fn eq(self, other: int) -> bool { self == other }
}

impl Eq for String {
    fn eq(self, other: String) -> bool { self == other }
}

impl Eq for bool {
    fn eq(self, other: bool) -> bool { self == other }
}

impl Ord for int {
    fn lt(self, other: int) -> bool { self < other }
}

impl Display for int {
    fn to_string(self) -> String { to_str(self) }
}

impl Display for String {
    fn to_string(self) -> String { self }
}

impl Display for bool {
    fn to_string(self) -> String { to_str(self) }
}

impl<A: Eq> Eq for Vec<A> {
    fn eq(self, other: Vec<A>) -> bool {
        if len(self) != len(other) { return false }
        for i in 0..len(self) {
            if i < len(other) { if !self[i].eq(other[i]) { return false } }
        }
        true
    }
}

impl<A: Display> Display for Vec<A> {
    fn to_string(self) -> String {
        let mut s = '['
        for i in 0..len(self) {
            if i > 0 { s = s + ', ' }
            s = s + self[i].to_string()
        }
        s + ']'
    }
}

impl<A: Eq> Eq for Option<A> {
    fn eq(self, other: Option<A>) -> bool {
        match self {
            None -> match other {
                None -> true
                Some(y) -> false
            }
            Some(x) -> match other {
                None -> false
                Some(y) -> x.eq(y)
            }
        }
    }
}

impl<A: Display> Display for Option<A> {
    fn to_string(self) -> String {
        match self {
            None -> 'None'
            Some(x) -> 'Some(' + x.to_string() + ')'
        }
    }
}

fn max<A: Ord>(a: A, b: A) -> A {
    if a.lt(b) { b } else { a }
}

fn min<A: Ord>(a: A, b: A) -> A {
    if b.lt(a) { b } else { a }
}

fn contains<A: Eq>(xs: Vec<A>, x: A) -> bool {
    for y in xs {
        if x.eq(y) { return true }
    }
    false
}

// ---- テスト ----

// 条件が偽なら実行時エラーにする（hibana test では、そのテストが失敗する）
fn assert(ok: bool) {
    if !ok { panic('assert が失敗した') }
}

fn assert_eq<A: Eq + Display>(got: A, want: A) {
    if got != want { panic('assert_eq が失敗した: ' + got.to_string() + ' と ' + want.to_string()) }
}

// 中の値。None なら実行時エラーにする
fn Option::unwrap<A>(o: Option<A>) -> A {
    match o {
        Some(x) -> x
        None -> panic('None を unwrap した')
    }
}

fn unwrap_or<A>(o: Option<A>, d: A) -> A {
    match o {
        Some(x) -> x
        None -> d
    }
}

// ---- Vec の高階関数 ----

// f の要求を R として、そのまま要求する（要求の多相）
fn map<A, B, R: Effect>(xs: Vec<A>, f: (A) -> B with R) -> Vec<B> with R {
    let mut out: Vec<B> = []
    for x in xs { out = push(out, f(x)) }
    out
}

fn filter<A, R: Effect>(xs: Vec<A>, f: (A) -> bool with R) -> Vec<A> with R {
    let mut out: Vec<A> = []
    for x in xs {
        if f(x) { out = push(out, x) }
    }
    out
}

fn fold<A, B, R: Effect>(xs: Vec<A>, init: B, f: (B, A) -> B with R) -> B with R {
    let mut acc = init
    for x in xs { acc = f(acc, x) }
    acc
}

fn concat<A>(a: Vec<A>, b: Vec<A>) -> Vec<A> {
    let mut out = a
    for x in b { out = push(out, x) }
    out
}

fn reverse<A>(xs: Vec<A>) -> Vec<A> {
    let mut out: Vec<A> = []
    for x in xs { out = concat([x], out) }
    out
}

// 先頭から n 個
fn take<A>(xs: Vec<A>, n: int) -> Vec<A> {
    let mut out: Vec<A> = []
    for i in 0..len(xs) {
        if i < n { out = push(out, xs[i]) }
    }
    out
}

// 先頭の n 個を除いた残り
fn drop<A>(xs: Vec<A>, n: int) -> Vec<A> {
    let mut out: Vec<A> = []
    for i in 0..len(xs) {
        if i >= n { out = push(out, xs[i]) }
    }
    out
}

fn last<A>(xs: Vec<A>) -> Option<A> {
    if len(xs) > 0 { Some(xs[len(xs) - 1]) } else { None }
}

// a 以上 b 未満の整数
fn range(a: int, b: int) -> Vec<int> {
    let mut out: Vec<int> = []
    for i in a..b { out = push(out, i) }
    out
}

fn sum(xs: Vec<int>) -> int {
    let mut s = 0
    for x in xs { s = s + x }
    s
}

fn some<A, R: Effect>(xs: Vec<A>, f: (A) -> bool with R) -> bool with R {
    for x in xs {
        if f(x) { return true }
    }
    false
}

fn every<A, R: Effect>(xs: Vec<A>, f: (A) -> bool with R) -> bool with R {
    for x in xs {
        if !f(x) { return false }
    }
    true
}

fn find<A, R: Effect>(xs: Vec<A>, f: (A) -> bool with R) -> Option<A> with R {
    for x in xs {
        if f(x) { return Some(x) }
    }
    None
}

// ---- 要求 ----

// 失敗。with Fail<E> で要求し、fail(e) で失敗する。提供する側は exit で抜けるか、try で Result にする
trait Fail<E> {
    fn fail(self, e: E) -> never
}

// f の失敗を Err に、成功を Ok にする。f のほかの要求 R はそのまま要求する
fn try<T, E, R: Effect>(f: () -> T with Fail<E>, R) -> Result<T, E> with R {
    with Fail(fn(e) { exit Err(e) }) {
        Ok(f())
    }
}

// ---- 非同期 ----

// 2 つの値の組
struct Pair<A, B> { first: A, second: B }

// 種類の違う 2 つの Async の値を並行に走らせ、両方が終わるまで待つ。最初の失敗で戻る
fn join<A, B>(a: Async<A>, b: Async<B>) -> Pair<A, B> with Async {
    let ra: Ref<Option<A>> = new_ref(None)
    let rb: Ref<Option<B>> = new_ref(None)
    all([join_into(a, ra), join_into(b, rb)]).await
    Pair { first: read(ra).unwrap(), second: read(rb).unwrap() }
}

fn join_into<T>(a: Async<T>, r: Ref<Option<T>>) with Async {
    write(r, Some(a.await))
}

// spawn で走らせた処理。終わったら f を呼ぶ（f は効果を持たない）
fn Task::then<T>(t: Task<T>, f: (T) -> unit) {
    task_then(t, f)
}

// 終わっていれば結果を Some で返す
fn Task::peek<T>(t: Task<T>) -> Option<T> {
    if task_done(t) { Some(task_value(t)) } else { None }
}

// ---- 文字列 ----

// xs.join(sep)。要素の間に sep を挟んでつなぐ（関数の join は Async の合流）
fn Vec::join(xs: Vec<String>, sep: String) -> String {
    let mut s = ''
    for i in 0..len(xs) {
        if i > 0 { s = s + sep }
        s = s + xs[i]
    }
    s
}

fn repeat(s: String, n: int) -> String {
    let mut r = ''
    for i in 0..n { r = r + s }
    r
}

fn ends_with(s: String, w: String) -> bool {
    starts_at(s, len(s) - len(w), w)
}

// s の中に w があるか
fn includes(s: String, w: String) -> bool {
    for i in 0..len(s) + 1 {
        if starts_at(s, i, w) { return true }
    }
    false
}

fn is_space(c: String) -> bool { c == ' ' || c == '\n' || c == '\t' || c == '\r' }

// 前後の空白を除く
fn trim(s: String) -> String {
    let mut a = 0
    let mut b = len(s)
    while a < b && is_space(char_at(s, a)) { a = a + 1 }
    while b > a && is_space(char_at(s, b - 1)) { b = b - 1 }
    slice(s, a, b)
}

// from を to に置き換える（from が空なら変えない）
fn replace(s: String, from: String, to: String) -> String {
    if len(from) == 0 { return s }
    let mut out = ''
    let mut i = 0
    while i < len(s) {
        if starts_at(s, i, from) {
            out = out + to
            i = i + len(from)
        } else {
            out = out + char_at(s, i)
            i = i + 1
        }
    }
    out
}

fn lines(s: String) -> Vec<String> { split(s, '\n') }

// ---- Map（文字列のキーから値へ） ----

// ponytail: 線形探索。大きくなるならハッシュ表にする
struct Entry<V> { key: String, value: V }
struct Map<V> { entries: Vec<Entry<V>> }

fn empty<V>() -> Map<V> {
    let es: Vec<Entry<V>> = []
    Map { entries: es }
}

fn get<V>(m: Map<V>, key: String) -> Option<V> {
    let mut r: Option<V> = None
    for e in m.entries {
        if e.key == key { r = Some(e.value) }
    }
    r
}

// key の値を value にした新しい Map
fn set<V>(m: Map<V>, key: String, value: V) -> Map<V> {
    let mut es: Vec<Entry<V>> = []
    for e in m.entries {
        if e.key != key { es = push(es, e) }
    }
    Map { entries: push(es, Entry { key: key, value: value }) }
}

fn keys<V>(m: Map<V>) -> Vec<String> {
    let mut ks: Vec<String> = []
    for e in m.entries { ks = push(ks, e.key) }
    ks
}

// ---- JSON ----

enum Json { JNull, JBool(b: bool), JInt(n: int), JStr(s: String), JArr(items: Vec<Json>), JObj(fields: Vec<Field>) }
struct Field { key: String, value: Json }

// JSON の文字列リテラル
fn quote_str(s: String) -> String {
    let mut out = '"'
    for i in 0..len(s) {
        let c = slice(s, i, i + 1)
        if c == '"' {
            out = out + '\\"'
        } else if c == '\\' {
            out = out + '\\\\'
        } else if c == '\n' {
            out = out + '\\n'
        } else if c == '\r' {
            out = out + '\\r'
        } else if c == '\t' {
            out = out + '\\t'
        } else if 0 <= code(c) && code(c) < 32 {
            let digits = '0123456789abcdef'
            out = out + '\\u00' + char_at(digits, code(c) / 16) + char_at(digits, code(c) % 16)
        } else {
            out = out + c
        }
    }
    out + '"'
}

fn stringify(j: Json) -> String {
    match j {
        JNull -> 'null'
        JBool(b) -> b.to_string()
        JInt(n) -> n.to_string()
        JStr(s) -> quote_str(s)
        JArr(items) -> {
            let mut out = '['
            let mut first = true
            for x in items {
                if !first { out = out + ',' }
                out = out + stringify(x)
                first = false
            }
            out + ']'
        }
        JObj(fields) -> {
            let mut out = '{'
            let mut first = true
            for f in fields {
                if !first { out = out + ',' }
                out = out + quote_str(f.key) + ':' + stringify(f.value)
                first = false
            }
            out + '}'
        }
    }
}

// ---- JSON を読む ----

// 読んだ値と、続きの位置。ok が false なら読めなかった
struct Parsed { ok: bool, value: Json, pos: int }

fn no_parse() -> Parsed {
    Parsed { ok: false, value: JNull, pos: 0 }
}

// i 文字目（範囲外なら ''）
fn char_at(s: String, i: int) -> String {
    if 0 <= i && i < len(s) { slice(s, i, i + 1) } else { '' }
}

fn skip_ws(s: String, i: int) -> int {
    let mut j = i
    while char_at(s, j) == ' ' || char_at(s, j) == '\n' || char_at(s, j) == '\t' || char_at(s, j) == '\r' {
        j = j + 1
    }
    j
}

// 数字なら 0 から 9、それ以外は -1
fn digit(c: String) -> int {
    let ds = '0123456789'
    let mut r = -1
    for k in 0..10 {
        if slice(ds, k, k + 1) == c { r = k }
    }
    r
}

fn starts_at(s: String, i: int, w: String) -> bool {
    0 <= i && i + len(w) <= len(s) && slice(s, i, i + len(w)) == w
}

// ponytail: 整数だけを読む（小数・指数は読めない）
fn parse_json_int(s: String, i: int) -> Parsed {
    let mut j = i
    let neg = char_at(s, j) == '-'
    if neg { j = j + 1 }
    let start = j
    let mut n = 0
    let mut overflow = false
    while digit(char_at(s, j)) >= 0 {
        let d = digit(char_at(s, j))
        // 負数は負のまま蓄積し、int の最小値を絶対値なしで表す。
        if !overflow {
            if neg {
                if n < -922337203685477580 || (n == -922337203685477580 && d > 8) {
                    overflow = true
                } else {
                    n = n * 10 - d
                }
            } else {
                if n > 922337203685477580 || (n == 922337203685477580 && d > 7) {
                    overflow = true
                } else {
                    n = n * 10 + d
                }
            }
        }
        j = j + 1
    }
    if j == start || overflow {
        no_parse()
    } else {
        Parsed { ok: true, value: JInt(n), pos: j }
    }
}

// JSON の Unicode エスケープを4桁読む。途中で切れているものも拒否する。
fn json_hex4(s: String, i: int) -> int {
    let mut n = 0
    for k in 0..4 {
        let c = char_at(s, i + k)
        let mut value = -1
        let lower = '0123456789abcdef'
        let upper = '0123456789ABCDEF'
        for h in 0..16 {
            if c == char_at(lower, h) || c == char_at(upper, h) { value = h }
        }
        if value < 0 { return -1 }
        n = n * 16 + value
    }
    n
}

fn parse_string(s: String, i: int) -> Parsed {
    let mut j = i + 1
    let mut out = ''
    let mut done = false
    let mut bad = false
    while !done && !bad {
        let c = char_at(s, j)
        if c == '' {
            bad = true
        } else if c == '"' {
            done = true
            j = j + 1
        } else if c == '\\' {
            let e = char_at(s, j + 1)
            if e == 'u' {
                let high = json_hex4(s, j + 2)
                if high < 0 || (56320 <= high && high <= 57343) {
                    bad = true
                } else if 55296 <= high && high <= 56319 {
                    let low = json_hex4(s, j + 8)
                    if char_at(s, j + 6) != '\\' || char_at(s, j + 7) != 'u' || low < 56320 || low > 57343 {
                        bad = true
                    } else {
                        out = out + char_from_codepoint(65536 + (high - 55296) * 1024 + low - 56320)
                        j = j + 12
                    }
                } else {
                    out = out + char_from_codepoint(high)
                    j = j + 6
                }
            } else {
                if e == 'n' {
                    out = out + '\n'
                } else if e == 'r' {
                    out = out + '\r'
                } else if e == 't' {
                    out = out + '\t'
                } else if e == 'b' {
                    out = out + char_from_codepoint(8)
                } else if e == 'f' {
                    out = out + char_from_codepoint(12)
                } else if e == '"' || e == '\\' || e == '/' {
                    out = out + e
                } else {
                    bad = true
                }
                j = j + 2
            }
        } else if code(c) < 32 {
            bad = true
        } else {
            out = out + c
            j = j + 1
        }
    }
    if bad { no_parse() } else { Parsed { ok: true, value: JStr(out), pos: j } }
}

fn parse_value(s: String, i0: int) -> Parsed {
    let i = skip_ws(s, i0)
    let c = char_at(s, i)
    if starts_at(s, i, 'null') {
        Parsed { ok: true, value: JNull, pos: i + 4 }
    } else if starts_at(s, i, 'true') {
        Parsed { ok: true, value: JBool(true), pos: i + 4 }
    } else if starts_at(s, i, 'false') {
        Parsed { ok: true, value: JBool(false), pos: i + 5 }
    } else if c == '"' {
        parse_string(s, i)
    } else if c == '[' {
        parse_array(s, i + 1)
    } else if c == '{' {
        parse_object(s, i + 1)
    } else {
        parse_json_int(s, i)
    }
}

// [ の後ろから読む
fn parse_array(s: String, i: int) -> Parsed {
    let mut items: Vec<Json> = []
    let mut j = skip_ws(s, i)
    let mut done = char_at(s, j) == ']'
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
            if c == ',' {
                j = j + 1
            } else if c == ']' {
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
fn parse_object(s: String, i: int) -> Parsed {
    let mut fields: Vec<Field> = []
    let mut j = skip_ws(s, i)
    let mut done = char_at(s, j) == '}'
    if done { j = j + 1 }
    let mut bad = false
    while !done && !bad {
        let k = parse_string(s, skip_ws(s, j))
        let colon = skip_ws(s, k.pos)
        if char_at(s, skip_ws(s, j)) != '"' || !k.ok || char_at(s, colon) != ':' {
            bad = true
        } else {
            let v = parse_value(s, colon + 1)
            if !v.ok {
                bad = true
            } else {
                match k.value {
                    JStr(key) -> { fields = push(fields, Field { key: key, value: v.value }) }
                    _ -> { bad = true }
                }
                j = skip_ws(s, v.pos)
                let c = char_at(s, j)
                if c == ',' {
                    j = j + 1
                } else if c == '}' {
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
fn parse_json(s: String) -> Option<Json> {
    let p = parse_value(s, 0)
    if p.ok && skip_ws(s, p.pos) == len(s) { Some(p.value) } else { None }
}

// オブジェクトのフィールド。無ければ None
fn field(j: Json, key: String) -> Option<Json> {
    let mut r: Option<Json> = None
    match j {
        JObj(fs) -> {
            for f in fs {
                if f.key == key { r = Some(f.value) }
            }
        }
        _ -> {}
    }
    r
}

// 文字列なら、その値。違えば d
fn str_or(j: Option<Json>, d: String) -> String {
    match j {
        Some(v) -> match v {
            JStr(s) -> s
            _ -> d
        }
        None -> d
    }
}

// 整数なら、その値。違えば d
fn int_or(j: Option<Json>, d: int) -> int {
    match j {
        Some(v) -> match v {
            JInt(n) -> n
            _ -> d
        }
        None -> d
    }
}
