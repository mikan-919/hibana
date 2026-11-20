// 標準ライブラリ。use std -> * で修飾なしに使う

// ---- Option と Result ----

pub enum Option<A> { None, Some(value: A) }
pub enum Result<A, E> { Ok(value: A), Err(error: E) }

// 符号 - と10進数字を読み、文字列全体が整数かを検査する
pub fn parse_int(s: String) -> Result<int, String> with {} {
    let cs = chars(s)
    let parsed = parse_json_int(cs, 0)
    if parsed.ok && parsed.pos == len(cs) {
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
pub trait Eq {
    fn eq(self, other: Self) -> bool with {}
}

// 順序。a.lt(b) は a が b より前
pub trait Ord: Eq {
    fn lt(self, other: Self) -> bool with {}
}

// 文字列にする。実装がある型では .to_string() と put がこれを使う
pub trait Display {
    fn to_string(self) -> String with {}
}

impl Eq for int {
    fn eq(self, other: int) -> bool { self == other }
}

impl Eq for number {
    fn eq(self, other: number) -> bool { self == other }
}

impl Ord for number {
    fn lt(self, other: number) -> bool { self < other }
}

impl Display for number {
    fn to_string(self) -> String { to_str(self) }
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

pub fn max<A: Ord>(a: A, b: A) -> A with {} {
    if a.lt(b) { b } else { a }
}

pub fn min<A: Ord>(a: A, b: A) -> A with {} {
    if b.lt(a) { b } else { a }
}

pub fn contains<A: Eq>(xs: Vec<A>, x: A) -> bool with {} {
    for y in xs {
        if x.eq(y) { return true }
    }
    false
}

// ---- テスト ----

// 条件が偽なら実行時エラーにする（hibana test では、そのテストが失敗する）
pub fn assert(ok: bool) with {} {
    if !ok { panic('assert が失敗した') }
}

pub fn assert_eq<A: Eq + Display>(got: A, want: A) with {} {
    if got != want { panic('assert_eq が失敗した: ' + got.to_string() + ' と ' + want.to_string()) }
}

// 中の値。None なら実行時エラーにする
pub fn Option::unwrap<A>(o: Option<A>) -> A with {} {
    match o {
        Some(x) -> x
        None -> panic('None を unwrap した')
    }
}

pub fn unwrap_or<A>(o: Option<A>, d: A) -> A with {} {
    match o {
        Some(x) -> x
        None -> d
    }
}

// ---- Vec の高階関数 ----

// f の要求を R として、そのまま要求する（要求の多相）
pub fn map<A, B, R: Effect>(xs: Vec<A>, f: (A) -> B with R) -> Vec<B> with R {
    let mut out: Vec<B> = []
    for x in xs { out = push(out, f(x)) }
    out
}

pub fn filter<A, R: Effect>(xs: Vec<A>, f: (A) -> bool with R) -> Vec<A> with R {
    let mut out: Vec<A> = []
    for x in xs {
        if f(x) { out = push(out, x) }
    }
    out
}

pub fn fold<A, B, R: Effect>(xs: Vec<A>, init: B, f: (B, A) -> B with R) -> B with R {
    let mut acc = init
    for x in xs { acc = f(acc, x) }
    acc
}

pub fn concat<A>(a: Vec<A>, b: Vec<A>) -> Vec<A> with {} {
    let mut out = a
    for x in b { out = push(out, x) }
    out
}

pub fn reverse<A>(xs: Vec<A>) -> Vec<A> with {} {
    let mut out: Vec<A> = []
    let mut i = len(xs) - 1
    while 0 <= i && i < len(xs) {
        out = push(out, xs[i])
        i = i - 1
    }
    out
}

// 先頭から n 個
pub fn take<A>(xs: Vec<A>, n: int) -> Vec<A> with {} {
    let mut out: Vec<A> = []
    for i in 0..len(xs) {
        if i < n { out = push(out, xs[i]) }
    }
    out
}

// 先頭の n 個を除いた残り
pub fn drop<A>(xs: Vec<A>, n: int) -> Vec<A> with {} {
    let mut out: Vec<A> = []
    for i in 0..len(xs) {
        if i >= n { out = push(out, xs[i]) }
    }
    out
}

pub fn last<A>(xs: Vec<A>) -> Option<A> with {} {
    if len(xs) > 0 { Some(xs[len(xs) - 1]) } else { None }
}

// a 以上 b 未満の整数
pub fn range(a: int, b: int) -> Vec<int> with {} {
    let mut out: Vec<int> = []
    for i in a..b { out = push(out, i) }
    out
}

pub fn sum(xs: Vec<int>) -> int with {} {
    let mut s = 0
    for x in xs { s = s + x }
    s
}

pub fn some<A, R: Effect>(xs: Vec<A>, f: (A) -> bool with R) -> bool with R {
    for x in xs {
        if f(x) { return true }
    }
    false
}

pub fn every<A, R: Effect>(xs: Vec<A>, f: (A) -> bool with R) -> bool with R {
    for x in xs {
        if !f(x) { return false }
    }
    true
}

pub fn find<A, R: Effect>(xs: Vec<A>, f: (A) -> bool with R) -> Option<A> with R {
    for x in xs {
        if f(x) { return Some(x) }
    }
    None
}

// ---- 要求 ----

// 失敗。with Fail<E> で要求し、fail(e) で失敗する。提供する側は exit で抜けるか、try で Result にする
pub trait Fail<E> {
    fn fail(self, e: E) -> never with {}
}

// f の失敗を Err に、成功を Ok にする。f のほかの要求 R はそのまま要求する
pub fn try<T, E, R: Effect>(f: () -> T with Fail<E>, R) -> Result<T, E> with R {
    with Fail(fn(e) { exit Err(e) }) {
        Ok(f())
    }
}

trait IntOverflow {
    fn raise_overflow(self) -> never with {}
}

pub fn try_overflow<T, R: Effect>(f: () -> T with R) -> Result<T, String> with R {
    with IntOverflow(fn() { exit Err('オーバーフロー') }) {
        Ok(f())
    }
}

// ---- 非同期 ----

// 2 つの値の組
pub struct Pair<A, B> { first: A, second: B }

// 種類の違う 2 つの Async の値を並行に走らせ、両方が終わるまで待つ。最初の失敗で戻る
pub fn join<A, B>(a: Async<A>, b: Async<B>) -> Pair<A, B> with Async {
    let ra: Ref<Option<A>> = new_ref(None)
    let rb: Ref<Option<B>> = new_ref(None)
    all([join_into(a, ra), join_into(b, rb)]).await
    Pair { first: read(ra).unwrap(), second: read(rb).unwrap() }
}

pub fn join_into<T>(a: Async<T>, r: Ref<Option<T>>) with Async {
    write(r, Some(a.await))
}

// spawn で走らせた処理。終わったら f を呼ぶ（f は効果を持たない）
pub fn Task::then<T>(t: Task<T>, f: (T) -> unit) with {} {
    task_then(t, f)
}

// 終わっていれば結果を Some で返す
pub fn Task::peek<T>(t: Task<T>) -> Option<T> with {} {
    if task_done(t) { Some(task_value(t)) } else { None }
}

// ---- 文字列 ----

// xs.join(sep)。要素の間に sep を挟んでつなぐ（関数の join は Async の合流）。
// 隣どうしをつなぐ段を繰り返す。各段で全体を 1 回写すので、長さの合計 N に対して O(N log N)
pub fn Vec::join(xs: Vec<String>, sep: String) -> String with {} {
    let mut parts: Vec<String> = xs
    while len(parts) > 1 {
        let mut next: Vec<String> = []
        let mut i = 0
        while i < len(parts) {
            if i + 1 < len(parts) {
                next = push(next, at(parts, i) + sep + at(parts, i + 1))
            } else {
                next = push(next, at(parts, i))
            }
            i = i + 2
        }
        parts = next
    }
    if len(parts) > 0 { parts[0] } else { '' }
}

pub fn repeat(s: String, n: int) -> String with {} {
    let mut r: Vec<String> = []
    for i in 0..n { r = push(r, s) }
    r.join('')
}

pub fn ends_with(s: String, w: String) -> bool with {} {
    starts_at(s, len(s) - len(w), w)
}

// s の中に w があるか
pub fn includes(s: String, w: String) -> bool with {} {
    for i in 0..len(s) + 1 {
        if starts_at(s, i, w) { return true }
    }
    false
}

pub fn is_space(c: String) -> bool with {} { c == ' ' || c == '\n' || c == '\t' || c == '\r' }

// 前後の空白を除く
pub fn trim(s: String) -> String with {} {
    let cs = chars(s)
    let mut a = 0
    let mut b = len(cs)
    while a < b && is_space(at(cs, a)) { a = a + 1 }
    while b > a && is_space(at(cs, b - 1)) { b = b - 1 }
    slice(s, a, b)
}

// from を to に置き換える（from が空なら変えない）
pub fn replace(s: String, from: String, to: String) -> String with {} {
    if len(from) == 0 { return s }
    split(s, from).join(to)
}

pub fn lines(s: String) -> Vec<String> with {} { split(s, '\n') }

// i 文字目（範囲外なら ''）
pub fn char_at(s: String, i: int) -> String with {} {
    if 0 <= i && i < len(s) { slice(s, i, i + 1) } else { '' }
}

// chars で分けた文字の並びの i 番目（範囲外なら ''）
pub fn at(cs: Vec<String>, i: int) -> String with {} {
    if 0 <= i && i < len(cs) { cs[i] } else { '' }
}

pub fn starts_at(s: String, i: int, w: String) -> bool with {} {
    0 <= i && i + len(w) <= len(s) && slice(s, i, i + len(w)) == w
}

// ---- Map（文字列のキーから値へ） ----

// キーのハッシュで分けたバケットの表。set は新しい Map を返し、元の Map は変わらない。
// keys の順は最後に set した順。order に set したキーを積み、Entry の seq がそのキーの最新の位置を指す。
// 古い位置が生きている数の 2 倍を超えるか、キーがバケットの数の 2 倍を超えると作り直す
// ponytail: set はバケットの並びと order を写すので O(キーの数)。get は O(1)。永続的な木（HAMT）にすれば set も O(log n)
pub struct Entry<V> { key: String, value: V, seq: int }
pub struct Map<V> { buckets: Vec<Vec<Entry<V>>>, order: Vec<String>, count: int }

pub fn empty<V>() -> Map<V> with {} {
    let order: Vec<String> = []
    Map { buckets: no_buckets(8), order: order, count: 0 }
}

pub fn no_buckets<V>(n: int) -> Vec<Vec<Entry<V>>> with {} {
    let mut bs: Vec<Vec<Entry<V>>> = []
    for i in 0..n {
        let b: Vec<Entry<V>> = []
        bs = push(bs, b)
    }
    bs
}

pub fn hash_key(key: String) -> int with {} {
    let mut h = 0
    for c in chars(key) { h = (h * 31 + code(c) + 1) % 1000000007 }
    h
}

// key の Entry。無ければ None
pub fn find_entry<V>(m: Map<V>, key: String) -> Option<Entry<V>> with {} {
    let bs = m.buckets
    let n = len(bs)
    let mut r: Option<Entry<V>> = None
    if n > 0 {
        let i = hash_key(key) % n
        if 0 <= i && i < len(bs) {
            for e in bs[i] {
                if e.key == key { r = Some(e) }
            }
        }
    }
    r
}

pub fn get<V>(m: Map<V>, key: String) -> Option<V> with {} {
    match find_entry(m, key) {
        Some(e) -> Some(e.value)
        None -> None
    }
}

// key の値を value にした新しい Map
pub fn set<V>(m: Map<V>, key: String, value: V) -> Map<V> with {} {
    let fresh = match find_entry(m, key) {
        Some(_) -> 0
        None -> 1
    }
    let count = m.count + fresh
    let mut base = m
    if count > 2 * len(m.buckets) || len(m.order) > 2 * count + 16 { base = rebuild(m, 2 * count + 8) }
    let seq = len(base.order)
    let mut bs = base.buckets
    let n = len(bs)
    if n > 0 {
        let i = hash_key(key) % n
        if 0 <= i && i < len(bs) {
            let mut b: Vec<Entry<V>> = []
            for x in bs[i] {
                if x.key != key { b = push(b, x) }
            }
            bs = set_at(bs, i, push(b, Entry { key: key, value: value, seq: seq }))
        }
    }
    Map { buckets: bs, order: push(base.order, key), count: count }
}

// 生きている Entry を keys の順に、size 個のバケットへ入れ直す。seq は新しい order の位置にする
pub fn rebuild<V>(m: Map<V>, size: int) -> Map<V> with {} {
    let mut bs: Vec<Vec<Entry<V>>> = no_buckets(size)
    let mut order: Vec<String> = []
    for k in keys(m) {
        match find_entry(m, k) {
            Some(e) -> {
                let n = len(bs)
                if n > 0 {
                    let i = hash_key(k) % n
                    if 0 <= i && i < len(bs) {
                        bs = set_at(bs, i, push(bs[i], Entry { key: k, value: e.value, seq: len(order) }))
                    }
                }
                order = push(order, k)
            }
            None -> {}
        }
    }
    Map { buckets: bs, order: order, count: len(order) }
}

pub fn keys<V>(m: Map<V>) -> Vec<String> with {} {
    let mut ks: Vec<String> = []
    let mut p = 0
    for k in m.order {
        match find_entry(m, k) {
            Some(e) -> { if e.seq == p { ks = push(ks, k) } }
            None -> {}
        }
        p = p + 1
    }
    ks
}

// ---- JSON ----

pub enum Json { JNull, JBool(b: bool), JInt(n: int), JNum(n: number), JStr(s: String), JArr(items: Vec<Json>), JObj(fields: Vec<Field>) }
pub struct Field { key: String, value: Json }

// JSON の文字列リテラル
pub fn quote_str(s: String) -> String with {} {
    let mut out: Vec<String> = ['"']
    for c in chars(s) {
        if c == '"' {
            out = push(out, '\\"')
        } else if c == '\\' {
            out = push(out, '\\\\')
        } else if c == '\n' {
            out = push(out, '\\n')
        } else if c == '\r' {
            out = push(out, '\\r')
        } else if c == '\t' {
            out = push(out, '\\t')
        } else if 0 <= code(c) && code(c) < 32 {
            let digits = chars('0123456789abcdef')
            out = push(out, '\\u00' + at(digits, code(c) / 16) + at(digits, code(c) % 16))
        } else {
            out = push(out, c)
        }
    }
    out = push(out, '"')
    out.join('')
}

pub fn stringify(j: Json) -> String with {} {
    match j {
        JNull -> 'null'
        JBool(b) -> b.to_string()
        JInt(n) -> n.to_string()
        JNum(n) -> {
            let s = n.to_string()
            if includes(s, '.') || includes(s, 'e') || includes(s, 'E') { s } else { s + '.0' }
        }
        JStr(s) -> quote_str(s)
        JArr(items) -> {
            let mut out: Vec<String> = []
            for x in items { out = push(out, stringify(x)) }
            '[' + out.join(',') + ']'
        }
        JObj(fields) -> {
            let mut out: Vec<String> = []
            for f in fields { out = push(out, quote_str(f.key) + ':' + stringify(f.value)) }
            '{' + out.join(',') + '}'
        }
    }
}

// ---- JSON を読む ----

// 読み手は、最初に chars で分けた文字の並び cs と位置を受け取る。1 文字を読むのは O(1) で、全体は入力の長さに比例する

// 読んだ値と、続きの位置。ok が false なら読めなかった
pub struct Parsed { ok: bool, value: Json, pos: int }

pub fn no_parse() -> Parsed with {} {
    Parsed { ok: false, value: JNull, pos: 0 }
}

pub fn skip_ws(cs: Vec<String>, i: int) -> int with {} {
    let mut j = i
    while is_space(at(cs, j)) { j = j + 1 }
    j
}

// 数字なら 0 から 9、それ以外は -1
pub fn digit(c: String) -> int with {} {
    if len(c) == 1 && 48 <= code(c) && code(c) <= 57 { code(c) - 48 } else { -1 }
}

// cs の i 文字目から w が続くか
pub fn chars_at(cs: Vec<String>, i: int, w: String) -> bool with {} {
    let mut k = i
    for c in chars(w) {
        if at(cs, k) != c { return false }
        k = k + 1
    }
    true
}

// parse_int 用。小数点と指数は読まず、int の範囲を検査する。
pub fn parse_json_int(cs: Vec<String>, i: int) -> Parsed with {} {
    let mut j = i
    let neg = at(cs, j) == '-'
    if neg { j = j + 1 }
    let start = j
    let mut n = 0
    let mut overflow = false
    while digit(at(cs, j)) >= 0 {
        let d = digit(at(cs, j))
        // 負数は負のまま蓄積し、int の最小値を絶対値なしで表す。
        if !overflow {
            if neg {
                if n < -900719925474099 || (n == -900719925474099 && d > 1) {
                    overflow = true
                } else {
                    n = n * 10 - d
                }
            } else {
                if n > 900719925474099 || (n == 900719925474099 && d > 1) {
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

// 各要素を基数1000万の桁として、先頭が非ゼロの整数を比較する。ゼロは [0]。
pub fn json_digits_cmp(a: Vec<int>, b: Vec<int>) -> int with {} {
    if len(a) < len(b) { return -1 }
    if len(a) > len(b) { return 1 }
    for i in 0..len(a) {
        if i < len(b) {
            if a[i] < b[i] { return -1 }
            if a[i] > b[i] { return 1 }
        }
    }
    0
}

pub fn json_digits_mul(a: Vec<int>, factor: int) -> Vec<int> with {} {
    let mut out = a
    let mut carry = 0
    let mut i = len(out) - 1
    while 0 <= i && i < len(out) {
        let d = out[i] * factor + carry
        out = set_at(out, i, d % 10000000)
        carry = d / 10000000
        i = i - 1
    }
    if carry > 0 { concat([carry], out) } else { out }
}

pub fn json_digits_scale(a: Vec<int>, exponent: int) -> Vec<int> with {} {
    let mut factor = 1
    for i in 0..exponent % 7 { factor = factor * 10 }
    let mut out = json_digits_mul(a, factor)
    for i in 0..exponent / 7 { out = push(out, 0) }
    out
}

// a >= b の数字列を引き算する。
pub fn json_digits_sub(a: Vec<int>, b: Vec<int>) -> Vec<int> with {} {
    let mut out = a
    let mut borrow = 0
    let mut i = len(out) - 1
    while 0 <= i && i < len(out) {
        let j = i - len(out) + len(b)
        let mut d = out[i] - borrow
        if 0 <= j && j < len(b) { d = d - b[j] }
        borrow = 0
        if d < 0 {
            d = d + 10000000
            borrow = 1
        }
        out = set_at(out, i, d)
        i = i - 1
    }
    let mut start = 0
    while 0 <= start && start < len(out) - 1 && start < len(out) && out[start] == 0 { start = start + 1 }
    drop(out, start)
}

// 10進の整数比から仮数を53ビット取り出し、最近接・偶数丸めで number にする。
pub fn json_decimal(digits: Vec<int>, exponent: int, neg: bool) -> Option<Json> with {} {
    let mut start = 0
    while 0 <= start && start < len(digits) && digits[start] == 0 { start = start + 1 }
    let mut r = drop(digits, start)
    let mut value = 0.0
    if len(r) > 0 {
        let order = len(r) - 1 + exponent
        if order > 308 { return None }
        if order >= -324 {
            let mut grouped: Vec<int> = []
            let mut group = 0
            for i in 0..len(r) {
                group = group * 10 + r[i]
                if (len(r) - 1 - i) % 7 == 0 {
                    grouped = push(grouped, group)
                    group = 0
                }
            }
            r = grouped
            let mut s = [1]
            if exponent > 0 { r = json_digits_scale(r, exponent) }
            if exponent < 0 { s = json_digits_scale(s, -exponent) }
            let mut binary = 0
            while json_digits_cmp(r, s) < 0 {
                r = json_digits_mul(r, 2)
                binary = binary - 1
            }
            let mut twice = json_digits_mul(s, 2)
            while json_digits_cmp(r, twice) >= 0 {
                s = twice
                twice = json_digits_mul(s, 2)
                binary = binary + 1
            }
            if binary > 1023 { return None }
            let mut count = 53
            let mut shift = binary - 52
            if binary < -1022 {
                count = binary + 1075
                shift = -1074
            }
            if count >= 0 {
                let mut mantissa = 0
                for i in 0..count {
                    mantissa = mantissa * 2
                    if json_digits_cmp(r, s) >= 0 {
                        mantissa = mantissa + 1
                        r = json_digits_sub(r, s)
                    }
                    r = json_digits_mul(r, 2)
                }
                value = mantissa
                let rounding = json_digits_cmp(r, s)
                if rounding > 0 || (rounding == 0 && mantissa % 2 != 0) { value = value + 1.0 }
                if binary == 1023 && value == 9007199254740992.0 { return None }
                while shift > 0 {
                    value = value * 2.0
                    shift = shift - 1
                }
                while shift < 0 {
                    value = value / 2.0
                    shift = shift + 1
                }
            }
        }
    }
    if neg { value = -value }
    Some(JNum(value))
}

pub fn parse_json_number(cs: Vec<String>, i: int) -> Parsed with {} {
    let mut j = i
    let neg = at(cs, j) == '-'
    if neg { j = j + 1 }
    let start = j
    let mut digits: Vec<int> = []
    while digit(at(cs, j)) >= 0 {
        digits = push(digits, digit(at(cs, j)))
        j = j + 1
    }
    if j == start || (j > start + 1 && at(cs, start) == '0') { return no_parse() }
    let mut floating = false
    let mut fraction = 0
    if at(cs, j) == '.' {
        floating = true
        j = j + 1
        let point = j
        while digit(at(cs, j)) >= 0 {
            digits = push(digits, digit(at(cs, j)))
            j = j + 1
        }
        fraction = j - point
        if fraction == 0 { return no_parse() }
    }
    let mut exponent = 0
    if at(cs, j) == 'e' || at(cs, j) == 'E' {
        floating = true
        j = j + 1
        let negative = at(cs, j) == '-'
        if negative || at(cs, j) == '+' { j = j + 1 }
        let power = j
        // 仮数の桁数で打ち消せない指数は飽和させ、桁数に比例する処理で読み終える。
        while digit(at(cs, j)) >= 0 {
            if exponent < len(cs) + 400 { exponent = exponent * 10 + digit(at(cs, j)) }
            j = j + 1
        }
        if j == power { return no_parse() }
        if negative { exponent = -exponent }
    }
    if !floating { return parse_json_int(cs, i) }
    match json_decimal(digits, exponent - fraction, neg) {
        Some(value) -> Parsed { ok: true, value: value, pos: j }
        None -> no_parse()
    }
}

// JSON の Unicode エスケープを4桁読む。途中で切れているものも拒否する。
pub fn json_hex4(cs: Vec<String>, i: int) -> int with {} {
    let mut n = 0
    for k in 0..4 {
        let c = at(cs, i + k)
        let x = code(c)
        let mut value = -1
        if len(c) == 1 && 48 <= x && x <= 57 { value = x - 48 }
        if len(c) == 1 && 97 <= x && x <= 102 { value = x - 87 }
        if len(c) == 1 && 65 <= x && x <= 70 { value = x - 55 }
        if value < 0 { return -1 }
        n = n * 16 + value
    }
    n
}

pub fn parse_string(cs: Vec<String>, i: int) -> Parsed with {} {
    let mut j = i + 1
    let mut out: Vec<String> = []
    let mut done = false
    let mut bad = false
    while !done && !bad {
        let c = at(cs, j)
        if c == '' {
            bad = true
        } else if c == '"' {
            done = true
            j = j + 1
        } else if c == '\\' {
            let e = at(cs, j + 1)
            if e == 'u' {
                let high = json_hex4(cs, j + 2)
                if high < 0 || (56320 <= high && high <= 57343) {
                    bad = true
                } else if 55296 <= high && high <= 56319 {
                    let low = json_hex4(cs, j + 8)
                    if at(cs, j + 6) != '\\' || at(cs, j + 7) != 'u' || low < 56320 || low > 57343 {
                        bad = true
                    } else {
                        out = push(out, char_from_codepoint(65536 + (high - 55296) * 1024 + low - 56320))
                        j = j + 12
                    }
                } else {
                    out = push(out, char_from_codepoint(high))
                    j = j + 6
                }
            } else {
                if e == 'n' {
                    out = push(out, '\n')
                } else if e == 'r' {
                    out = push(out, '\r')
                } else if e == 't' {
                    out = push(out, '\t')
                } else if e == 'b' {
                    out = push(out, char_from_codepoint(8))
                } else if e == 'f' {
                    out = push(out, char_from_codepoint(12))
                } else if e == '"' || e == '\\' || e == '/' {
                    out = push(out, e)
                } else {
                    bad = true
                }
                j = j + 2
            }
        } else if code(c) < 32 {
            bad = true
        } else {
            out = push(out, c)
            j = j + 1
        }
    }
    if bad { no_parse() } else { Parsed { ok: true, value: JStr(out.join('')), pos: j } }
}

pub fn parse_value(cs: Vec<String>, i0: int) -> Parsed with {} {
    let i = skip_ws(cs, i0)
    let c = at(cs, i)
    if chars_at(cs, i, 'null') {
        Parsed { ok: true, value: JNull, pos: i + 4 }
    } else if chars_at(cs, i, 'true') {
        Parsed { ok: true, value: JBool(true), pos: i + 4 }
    } else if chars_at(cs, i, 'false') {
        Parsed { ok: true, value: JBool(false), pos: i + 5 }
    } else if c == '"' {
        parse_string(cs, i)
    } else if c == '[' {
        parse_array(cs, i + 1)
    } else if c == '{' {
        parse_object(cs, i + 1)
    } else {
        parse_json_number(cs, i)
    }
}

// [ の後ろから読む
pub fn parse_array(cs: Vec<String>, i: int) -> Parsed with {} {
    let mut items: Vec<Json> = []
    let mut j = skip_ws(cs, i)
    let mut done = at(cs, j) == ']'
    if done { j = j + 1 }
    let mut bad = false
    while !done && !bad {
        let p = parse_value(cs, j)
        if !p.ok {
            bad = true
        } else {
            items = push(items, p.value)
            j = skip_ws(cs, p.pos)
            let c = at(cs, j)
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
pub fn parse_object(cs: Vec<String>, i: int) -> Parsed with {} {
    let mut fields: Vec<Field> = []
    let mut j = skip_ws(cs, i)
    let mut done = at(cs, j) == '}'
    if done { j = j + 1 }
    let mut bad = false
    while !done && !bad {
        let k = parse_string(cs, skip_ws(cs, j))
        let colon = skip_ws(cs, k.pos)
        if at(cs, skip_ws(cs, j)) != '"' || !k.ok || at(cs, colon) != ':' {
            bad = true
        } else {
            let v = parse_value(cs, colon + 1)
            if !v.ok {
                bad = true
            } else {
                match k.value {
                    JStr(key) -> { fields = push(fields, Field { key: key, value: v.value }) }
                    _ -> { bad = true }
                }
                j = skip_ws(cs, v.pos)
                let c = at(cs, j)
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
pub fn parse_json(s: String) -> Option<Json> with {} {
    let cs = chars(s)
    let p = parse_value(cs, 0)
    if p.ok && skip_ws(cs, p.pos) == len(cs) { Some(p.value) } else { None }
}

// オブジェクトのフィールド。無ければ None
pub fn field(j: Json, key: String) -> Option<Json> with {} {
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
pub fn str_or(j: Option<Json>, d: String) -> String with {} {
    match j {
        Some(v) -> match v {
            JStr(s) -> s
            _ -> d
        }
        None -> d
    }
}

// 整数なら、その値。違えば d
pub fn int_or(j: Option<Json>, d: int) -> int with {} {
    match j {
        Some(v) -> match v {
            JInt(n) -> n
            _ -> d
        }
        None -> d
    }
}
