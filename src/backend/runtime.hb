// IR のランタイム。WASM とネイティブの両方で同じものを使う。
// 型検査は通さず、構文解析だけして IR に変換する。演算子はすべて値の中身（64 ビット整数）の演算になる。
// __load などの __ で始まる組み込みは、IR の命令に直接変換する（src/backend/ir.rs の prim）。
//
// 値の型の番号: 0 unit, 1 int, 2 bool, 3 String, 4 Vec, 5 type の値・構造体, 6 関数, 7 型, 8 Ref, 9 number（中身は f64 のビット列）
// 10 は GC の根に置く with の記録の並び（ランタイムの中だけで使う）
// ヒープ上の形（各欄は 8 バイト、値は型の番号と中身の 16 バイト）:
//   String: [長さ][UTF-8 のバイト列]
//   Vec: [要素数][値...]。位置の直前に [容量][所有の印] を置く（__arr_new）
//   type の値・構造体: [構築子の番号][フィールド数][値...]
//   関数: [関数の番号][写し取った変数の数][値...]
//   Ref: [値]
//   型: 中身は型の名前の String
//
// メモリの回収（GC）。__alloc で確保した区画は、その直前に [次の区画][大きさ << 8 | 種類 << 1 | 印] の 16 バイトを置き、
// すべての区画を 1 本の並び（+88）でつなぐ。種類は 0 中に値を持たない、1 Vec、2 構造体と関数、3 Ref、4 with の記録。
// 根は、シャドースタック（ir.rs が利用者の関数の局所変数を写す区画）と、中断の値（+32）と処理中の with（+48）。
// GC は利用者の関数の入口とループの先頭（ir.rs の安全な位置）でだけ動く。ランタイムの関数の中の生の位置は、
// GC の間に生きていない。ランタイムの関数から利用者の関数を呼ぶ所（__all・__perform）は、持っている値をシャドースタックに置く

// ランタイムの状態は __static() から 152 バイト:
//   +0 確保の次の位置、+8 区画の終わり、+16 実行時エラーの文字列（WASM だけ使う）
//   +24 中断の行き先（with の番号。0 は中断していない）、+32 中断で with に渡す値
//   +48 処理中のいちばん内側の with、+56 最後に付けた with の番号
//   +64 シャドースタックの先頭、+72 その始まり、+80 その終わり
//   +88 確保した区画の並び、+96 前回の GC の後に確保したバイト数、+104 GC を始めるバイト数
//   +112 これより前の位置は文字列定数（WASM は接続コードが書く。0 なら __static() の位置）
//   +120 空き区画の並びの表（1024 バイトまでは 16 バイトごと、それより大きい区画は 2 のべきごとの 128 本）
//   +128 印を付ける作業の並び、+136 その容量
//   +144 標準入力の区画（__input）
fn __alloc(n: int) -> int {
    let st = __static()
    if __load(st + 120) == 0 {
        __gc_init()
    }
    // 1024 バイトまでは 16 バイトごと、それ以上は 2 のべきに切り上げ、同じ大きさの空き区画を使う
    // ponytail: 区画を分けたりつないだりしない。大きい区画は最大で 2 倍の無駄がある
    let mut size = (n + 15) / 16 * 16
    if size == 0 {
        size = 16
    }
    if size > 1024 {
        let mut s = 1024
        while s < size {
            s = s * 2
        }
        size = s
    }
    let slot = __load(st + 120) + __size_class(size) * 8
    let mut b = __load(slot)
    if b == 0 {
        b = __bump(size + 16) + 16
    } else {
        __store(slot, __load(b - 16))
        __zero(b, size)
    }
    __store(b - 8, __shl(size, 8))
    __store(b - 16, __load(st + 88))
    __store(st + 88, b)
    __store(st + 96, __load(st + 96) + size + 16)
    b
}

// 空き区画の並びの表の番号。__alloc で切り上げた大きさを渡す
fn __size_class(size: int) -> int {
    if size <= 1024 {
        return size / 16
    }
    let mut c = 64
    let mut s = 1024
    while s < size {
        s = s * 2
        c = c + 1
    }
    c
}

fn __gc_init() {
    let st = __static()
    let table = __bump(1024)
    __zero(table, 1024)
    __store(st + 120, table)
    __store(st + 104, 4194304)
    if __load(st + 112) == 0 {
        __store(st + 112, st)
    }
}

// 区画から n バイトを切り出す。足りなければページを足す
fn __bump(n: int) -> int {
    let st = __static()
    let size = (n + 7) / 8 * 8
    let mut p = __load(st)
    let end = __load(st + 8)
    if p + size > end {
        let mut chunk = 16777216
        if size > chunk {
            chunk = size
        }
        let q = __pages(chunk)
        // 続きの領域なら、今の区画の残りもそのまま使う
        if q != end {
            p = q
        }
        __store(st + 8, q + chunk)
    }
    __store(st, p + size)
    p
}

fn __zero(p: int, n: int) {
    let mut i = 0
    while i < n {
        __store(p + i, 0)
        i = i + 8
    }
}

// 区画の種類を付ける（__alloc の直後に呼ぶ）
fn __gc_kind(p: int, kind: int) -> int {
    __store(p - 8, __or(__load(p - 8), __shl(kind, 1)))
    p
}

// 利用者の関数の局所変数 n 個を置くシャドースタックの区画。0 で埋める。戻る時に ir.rs が +64 を戻す
fn __frame_push(n: int) -> int {
    let st = __static()
    let mut sp = __load(st + 64)
    if sp == 0 {
        let size = 16777216
        sp = __bump(size)
        __store(st + 72, sp)
        __store(st + 80, sp + size)
    }
    let size = n * 16
    if sp + size > __load(st + 80) {
        __die('関数の呼び出しが深すぎる')
    }
    __zero(sp, size)
    __store(st + 64, sp + size)
    sp
}

// 値 v が指す区画に印を付け、作業の並びに積む。文字列定数と、中身が位置でない値は飛ばす
fn __gc_mark(v: int) {
    let tag = __tag(v)
    if tag < 3 || tag == 9 || tag > 10 {
        return
    }
    let mut b = __payload(v)
    if tag == 4 {
        b = b - 16
    }
    if b < __load(__static() + 112) {
        return
    }
    __gc_push(b)
}

fn __gc_push(b: int) {
    let h = __load(b - 8)
    if __and(h, 1) == 1 {
        return
    }
    __store(b - 8, __or(h, 1))
    if __and(__shr(h, 1), 127) == 0 {
        return
    }
    let st = __static()
    let mut list = __load(st + 128)
    let n = __load(list)
    if n + 1 >= __load(st + 136) {
        // 作業の並びを 2 倍にする。古い並びは捨てる
        // ponytail: 古い並びは回収しない（GC の並びの外）。大きくなる回数は少ない
        let cap = __load(st + 136) * 2
        let q = __bump(cap * 8)
        __copy(q, list, (n + 1) * 8)
        __store(st + 128, q)
        __store(st + 136, cap)
        list = q
    }
    __store(list + 8 + n * 8, b)
    __store(list, n + 1)
}

// 値の並び（p から n 個）に印を付ける
fn __gc_values(p: int, n: int) {
    let mut i = 0
    while i < n {
        __gc_mark(__getv(p + i * 16))
        i = i + 1
    }
}

fn __gc() {
    let st = __static()
    if __load(st + 128) == 0 {
        __store(st + 128, __bump(65536))
        __store(st + 136, 8191)
    }
    __store(__load(st + 128), 0)
    __gc_values(__load(st + 72), (__load(st + 64) - __load(st + 72)) / 16)
    __gc_mark(__getv(st + 32))
    if __load(st + 48) != 0 {
        __gc_push(__load(st + 48))
    }
    // 作業の並びは __gc_push が大きくすると移るので、毎回 +128 から読む
    while __load(__load(st + 128)) > 0 {
        let list = __load(st + 128)
        let n = __load(list) - 1
        __store(list, n)
        let b = __load(list + 8 + n * 8)
        let kind = __and(__shr(__load(b - 8), 1), 127)
        if kind == 1 {
            __gc_values(b + 24, __load(b + 16))
        }
        if kind == 2 {
            __gc_values(b + 16, __load(b + 8))
        }
        if kind == 3 {
            __gc_values(b, 1)
        }
        if kind == 4 {
            if __load(b) != 0 {
                __gc_push(__load(b))
            }
            __gc_values(b + 16, 2)
        }
    }
    // 印の無い区画を空きの並びに戻す
    let table = __load(st + 120)
    let mut live = 0
    let mut prev = 0
    let mut b = __load(st + 88)
    while b != 0 {
        let next = __load(b - 16)
        let h = __load(b - 8)
        let size = __shr(h, 8)
        if __and(h, 1) == 1 {
            __store(b - 8, h - 1)
            live = live + size + 16
            prev = b
        } else {
            if prev == 0 {
                __store(st + 88, next)
            } else {
                __store(prev - 16, next)
            }
            __store(b - 8, __shl(size, 8))
            let slot = table + __size_class(size) * 8
            __store(b - 16, __load(slot))
            __store(slot, b)
        }
        b = next
    }
    __store(st + 96, 0)
    let mut limit = 4194304
    if live > limit {
        limit = live
    }
    __store(st + 104, limit)
}

fn __die(msg: String) {
    __fail(msg)
}

fn __overflow() {
    __die('オーバーフロー')
}

fn __div_zero() {
    __die('0 で除算した')
}

// 利用者の整数演算。ランタイム内部の64ビット演算とは分ける。
fn __i53(n: int) -> int {
    if n > 9007199254740991 || n < -9007199254740991 { __overflow() }
    n
}
fn __iadd(a: int, b: int) -> int { __i53(a + b) }
fn __isub(a: int, b: int) -> int { __i53(a - b) }
fn __imul(a: int, b: int) -> int { __i53(a * b) }
fn __ineg(a: int) -> int { __i53(0 - a) }

// WASM 境界の f64 のビット列を安全整数へ変換する。
fn __int_in(bits: int) -> int {
    if __and(__shr(__payload(bits), 52), 2047) == 2047 { __die('安全整数の範囲外') }
    let v = __mkv(9, __payload(bits))
    if __f_lt(v, -9007199254740991.0) || __f_lt(9007199254740991.0, v) || !__f_eq(__f_floor(v), v) { __die('安全整数の範囲外') }
    __f_to_int(v)
}

// 指数が 52 を超えると、-1・0・1 以外は範囲外になる。-1・0・1 は偶奇だけで決まる
fn __pow(a: int, b: int) -> int {
    if b < 0 {
        __die('負の指数')
    }
    let mut n = b
    if n > 52 {
        if a > 1 || a < 0 - 1 {
            __overflow()
        }
        n = 52 + n % 2
    }
    let mut r = 1
    while n > 0 {
        r = __imul(r, a)
        n = n - 1
    }
    r
}

fn __copy(dst: int, src: int, n: int) {
    let mut i = 0
    while i + 8 <= n {
        __store(dst + i, __load(src + i))
        i = i + 8
    }
    while i < n {
        __store8(dst + i, __load8(src + i))
        i = i + 1
    }
}

// 文字列

fn __str_new(n: int) -> int {
    let p = __alloc(8 + n)
    __store(p, n)
    p
}

fn __bytes(src: int, n: int) -> String {
    let p = __str_new(n)
    __copy(p + 8, src, n)
    __mkv(3, p)
}

fn __byte(b: int) -> String {
    let p = __str_new(1)
    __store8(p + 8, b)
    __mkv(3, p)
}

fn __cat(a: String, b: String) -> String {
    let pa = __payload(a)
    let pb = __payload(b)
    let la = __load(pa)
    let lb = __load(pb)
    let p = __str_new(la + lb)
    __copy(p + 8, pa + 8, la)
    __copy(p + 8 + la, pb + 8, lb)
    __mkv(3, p)
}

// 数値の加算と文字列の連結。値の型の番号に応じて処理する
fn __add(a: String, b: String) -> String {
    if __tag(a) == 9 || __tag(b) == 9 { return __num_checked(__f_add(__num(a), __num(b))) }
    if __tag(a) == 1 { return __iadd(a, b) }
    __cat(a, b)
}

fn __num_checked(bits: int) -> number {
    if __and(__shr(__payload(bits), 52), 2047) == 2047 { __die('number の演算結果が有限でない') }
    __mkv(9, __payload(bits))
}

fn __num(v: int) -> number {
    if __tag(v) == 9 { return __num_checked(v) }
    __num_checked(__f_from_int(v))
}

fn __neg(v: int) -> int {
    if __tag(v) == 9 { return __num_checked(__f_neg(v)) }
    __ineg(v)
}
fn __sub(a: int, b: int) -> int {
    if __tag(a) == 9 || __tag(b) == 9 { return __num_checked(__f_sub(__num(a), __num(b))) }
    __isub(a, b)
}
fn __mul(a: int, b: int) -> int {
    if __tag(a) == 9 || __tag(b) == 9 { return __num_checked(__f_mul(__num(a), __num(b))) }
    __imul(a, b)
}
fn __div(a: int, b: int) -> int {
    if __tag(a) == 9 || __tag(b) == 9 {
        if __f_eq(__num(b), 0.0) { __div_zero() }
        return __num_checked(__f_div(__num(a), __num(b)))
    }
    a / b
}
fn __lt(a: int, b: int) -> bool {
    if __tag(a) == 9 || __tag(b) == 9 { return __f_lt(__num(a), __num(b)) }
    a < b
}
fn __le(a: int, b: int) -> bool {
    if __tag(a) == 9 || __tag(b) == 9 { return __f_le(__num(a), __num(b)) }
    a <= b
}
fn __gt(a: int, b: int) -> bool { __lt(b, a) }
fn __ge(a: int, b: int) -> bool { __le(b, a) }

fn __num_floor(v: number) -> int {
    let n = __f_floor(__num(v))
    if __f_lt(n, -9007199254740991.0) || __f_lt(9007199254740991.0, n) { __die('floor の結果が int の範囲外') }
    __f_to_int(n)
}
fn __num_sqrt(value: number) -> number {
    let v = __num(value)
    if __f_lt(v, 0.0) { __die('sqrt の引数が負') }
    __num_checked(__f_sqrt(v))
}

// ponytail: number の指数は整数のみ。小数の指数には対数・指数関数の実装が必要
fn __value_pow(left: int, right: int) -> int {
    if __tag(left) != 9 && __tag(right) != 9 { return __pow(left, right) }
    let a = __num(left)
    let b = __num(right)
    if !__f_eq(__f_floor(b), b) { __die('number の小数の指数は未対応') }
    let mut n = b
    let mut factor = a
    if __f_lt(n, 0.0) {
        n = __f_neg(n)
        if __f_eq(a, 0.0) { __die('number の演算結果が有限でない') }
        factor = __num_checked(__f_div(1.0, a))
    }
    let mut result = 1.0
    while !__f_eq(n, 0.0) {
        let half = __f_floor(__f_div(n, 2.0))
        if !__f_eq(__f_mul(half, 2.0), n) { result = __num_checked(__f_mul(result, factor)) }
        n = half
        if !__f_eq(n, 0.0) { factor = __num_checked(__f_mul(factor, factor)) }
    }
    result
}

// 仮数の整数除算で余りを求め、除算の結果が桁あふれする場合も扱う
fn __rem(left: int, right: int) -> int {
    if __tag(left) != 9 && __tag(right) != 9 { return left % right }
    let a = __num(left)
    let b = __num(right)
    let sign = __shl(__shr(__payload(a), 63), 63)
    let mut x = __and(__payload(a), 9223372036854775807)
    let y = __and(__payload(b), 9223372036854775807)
    if y == 0 { __div_zero() }
    if x < y { return a }
    if x == y { return __mkv(9, sign) }
    let mut ex = __shr(x, 52)
    let mut ey = __shr(y, 52)
    let mut my = __and(y, 4503599627370495)
    x = __and(x, 4503599627370495)
    if ex == 0 { ex = 1 } else { x = x + 4503599627370496 }
    if ey == 0 { ey = 1 } else { my = my + 4503599627370496 }
    while x < 4503599627370496 { x = x * 2; ex = ex - 1 }
    while my < 4503599627370496 { my = my * 2; ey = ey - 1 }
    while ex > ey {
        if x >= my { x = x - my }
        if x == 0 { return __mkv(9, sign) }
        x = x * 2
        ex = ex - 1
    }
    if x >= my { x = x - my }
    if x == 0 { return __mkv(9, sign) }
    while x < 4503599627370496 { x = x * 2; ex = ex - 1 }
    if ex > 0 { x = x - 4503599627370496 + __shl(ex, 52) } else { x = __shr(x, 1 - ex) }
    __mkv(9, __or(x, sign))
}

fn __str_eq(pa: int, pb: int) -> bool {
    let n = __load(pa)
    if n != __load(pb) {
        return false
    }
    let mut i = 0
    while i < n {
        if __load8(pa + 8 + i) != __load8(pb + 8 + i) {
            return false
        }
        i = i + 1
    }
    true
}

fn __utf8_len(p: int) -> int {
    let n = __load(p)
    let mut i = 0
    let mut c = 0
    while i < n {
        if __and(__load8(p + 8 + i), 192) != 128 {
            c = c + 1
        }
        i = i + 1
    }
    c
}

// f64 の境界を10進の整数比で保持する。40桁の基数10億で全 f64 と丸めの余白を収める
fn __big(n: int) -> int {
    let p = __alloc(328)
    let mut value = n
    let mut len = 0
    while len == 0 || value != 0 {
        __store(p + 8 + len * 8, value % 1000000000)
        value = value / 1000000000
        len = len + 1
    }
    __store(p, len)
    p
}
fn __big_copy(p: int) -> int {
    let q = __alloc(328)
    __copy(q, p, 8 + __load(p) * 8)
    q
}
fn __big_mul(p: int, factor: int) {
    let mut carry = 0
    let mut i = 0
    let n = __load(p)
    while i < n {
        let d = __load(p + 8 + i * 8) * factor + carry
        __store(p + 8 + i * 8, d % 1000000000)
        carry = d / 1000000000
        i = i + 1
    }
    if carry != 0 {
        if n >= 40 { __die('number の表示用整数が範囲外') }
        __store(p + 8 + n * 8, carry)
        __store(p, n + 1)
    }
}
fn __big_cmp(a: int, b: int) -> int {
    let na = __load(a)
    let nb = __load(b)
    if na < nb { return 0 - 1 }
    if na > nb { return 1 }
    let mut i = na
    while i > 0 {
        i = i - 1
        let x = __load(a + 8 + i * 8)
        let y = __load(b + 8 + i * 8)
        if x < y { return 0 - 1 }
        if x > y { return 1 }
    }
    0
}
fn __big_sub(a: int, b: int) {
    let n = __load(a)
    let mut borrow = 0
    let mut i = 0
    while i < n {
        let mut d = __load(a + 8 + i * 8) - borrow
        if i < __load(b) { d = d - __load(b + 8 + i * 8) }
        borrow = 0
        if d < 0 { d = d + 1000000000; borrow = 1 }
        __store(a + 8 + i * 8, d)
        i = i + 1
    }
    let mut len = n
    while len > 1 && __load(a + len * 8) == 0 { len = len - 1 }
    __store(a, len)
}

// 隣り合う f64 の中点を境界にして、元の値へ丸め戻せる最短の10進表記を選ぶ
fn __num_str(bits: int) -> String {
    let negative = __shr(bits, 63) != 0
    let absolute = __and(bits, 9223372036854775807)
    if absolute == 0 {
        if negative { return '-0' }
        return '0'
    }
    let exponent = __shr(absolute, 52)
    let fraction = __and(absolute, 4503599627370495)
    let mut mantissa = fraction
    let mut shift = 0 - 1076
    if exponent != 0 {
        mantissa = mantissa + 4503599627370496
        shift = exponent - 1077
    }
    let r = __big(mantissa * 4)
    let s = __big(1)
    let mut lower = 2
    if exponent > 1 && fraction == 0 { lower = 1 }
    let lo = __big(lower)
    let hi = __big(2)
    while shift > 0 {
        __big_mul(r, 2); __big_mul(lo, 2); __big_mul(hi, 2)
        shift = shift - 1
    }
    while shift < 0 { __big_mul(s, 2); shift = shift + 1 }
    let mut decimal = 0
    let ten_s = __big_copy(s)
    __big_mul(ten_s, 10)
    while __big_cmp(r, ten_s) >= 0 {
        __big_mul(s, 10); __big_mul(ten_s, 10)
        decimal = decimal + 1
    }
    while __big_cmp(r, s) < 0 {
        __big_mul(r, 10); __big_mul(lo, 10); __big_mul(hi, 10)
        decimal = decimal - 1
    }
    let digits = __alloc(24)
    let mut count = 0
    let mut done = false
    let inclusive = __and(mantissa, 1) == 0
    while !done {
        let mut digit = 0
        while __big_cmp(r, s) >= 0 { __big_sub(r, s); digit = digit + 1 }
        __store8(digits + count, digit)
        count = count + 1
        let low_cmp = __big_cmp(r, lo)
        let gap = __big_copy(s)
        __big_sub(gap, r)
        let high_cmp = __big_cmp(gap, hi)
        let low = low_cmp < 0 || (inclusive && low_cmp == 0)
        let high = high_cmp < 0 || (inclusive && high_cmp == 0)
        if low || high {
            let twice = __big_copy(r)
            __big_mul(twice, 2)
            let middle = __big_cmp(twice, s)
            let up = high && (!low || middle > 0 || (middle == 0 && digit % 2 != 0))
            if up {
                let mut pos = count - 1
                while pos > 0 && __load8(digits + pos) == 9 {
                    __store8(digits + pos, 0)
                    pos = pos - 1
                }
                if __load8(digits + pos) == 9 {
                    __store8(digits, 1)
                    decimal = decimal + 1
                } else { __store8(digits + pos, __load8(digits + pos) + 1) }
            }
            done = true
        } else {
            if count >= 17 { __die('number の表示を丸められない') }
            __big_mul(r, 10); __big_mul(lo, 10); __big_mul(hi, 10)
        }
    }
    while count > 1 && __load8(digits + count - 1) == 0 { count = count - 1 }
    let mut i = 0
    while i < count { __store8(digits + i, __load8(digits + i) + 48); i = i + 1 }
    let mut text = ''
    let point = decimal + 1
    if point <= 0 {
        text = '0.'
        let mut zeros = 0 - point
        while zeros > 0 { text = __cat(text, '0'); zeros = zeros - 1 }
        text = __cat(text, __bytes(digits, count))
    } else if point >= count {
        text = __bytes(digits, count)
        let mut zeros = point - count
        while zeros > 0 { text = __cat(text, '0'); zeros = zeros - 1 }
    } else {
        text = __cat(__cat(__bytes(digits, point), '.'), __bytes(digits + point, count - point))
    }
    if negative { text = __cat('-', text) }
    text
}

fn __int_str(n: int) -> String {
    let p = __str_new(20)
    let mut pos = 20
    let mut u = n
    let mut first = true
    while first || u != 0 {
        first = false
        let mut d = u % 10
        if d < 0 {
            d = 0 - d
        }
        pos = pos - 1
        __store8(p + 8 + pos, 48 + d)
        u = u / 10
    }
    if n < 0 {
        pos = pos - 1
        __store8(p + 8 + pos, 45)
    }
    __bytes(p + 8 + pos, 20 - pos)
}

// 値の比較と表示

fn __eq(a: int, b: int) -> bool {
    let t = __tag(a)
    if t != __tag(b) {
        if (t == 1 && __tag(b) == 9) || (t == 9 && __tag(b) == 1) { return __f_eq(__num(a), __num(b)) }
        return false
    }
    if t == 9 { return __f_eq(a, b) }
    let pa = __payload(a)
    let pb = __payload(b)
    if pa == pb {
        return true
    }
    if t == 3 || t == 7 {
        return __str_eq(pa, pb)
    }
    if t == 4 {
        let n = __load(pa)
        if n != __load(pb) {
            return false
        }
        let mut i = 0
        while i < n {
            if !__eq(__getv(pa + 8 + i * 16), __getv(pb + 8 + i * 16)) {
                return false
            }
            i = i + 1
        }
        return true
    }
    if t == 5 {
        if __load(pa) != __load(pb) {
            return false
        }
        let n = __load(pa + 8)
        let mut i = 0
        while i < n {
            if !__eq(__getv(pa + 16 + i * 16), __getv(pb + 16 + i * 16)) {
                return false
            }
            i = i + 1
        }
        return true
    }
    false
}

// 文字列の中の値として表示する。インタプリタ（Rust の {:?}）と同じ記号を逃がす
fn __quote(s: String) -> String {
    let p = __payload(s)
    let n = __load(p)
    let mut out = '"'
    let mut start = 0
    let mut i = 0
    while i < n {
        let c = __load8(p + 8 + i)
        let mut esc = ''
        if c == 34 {
            esc = '\\"'
        }
        if c == 92 {
            esc = '\\\\'
        }
        if c == 10 {
            esc = '\\n'
        }
        if c == 9 {
            esc = '\\t'
        }
        if c == 13 {
            esc = '\\r'
        }
        if c == 0 {
            esc = '\\0'
        }
        if __load(__payload(esc)) != 0 {
            out = __cat(__cat(out, __bytes(p + 8 + start, i - start)), esc)
            start = i + 1
        }
        i = i + 1
    }
    __cat(__cat(out, __bytes(p + 8 + start, n - start)), '"')
}

fn __join_shown(p: int, n: int) -> String {
    let mut out = ''
    let mut i = 0
    while i < n {
        if i > 0 {
            out = __cat(out, ', ')
        }
        out = __cat(out, __show(__getv(p + i * 16)))
        i = i + 1
    }
    out
}

fn __show(v: int) -> String {
    let t = __tag(v)
    let p = __payload(v)
    if t == 0 {
        return '()'
    }
    if t == 1 {
        return __int_str(p)
    }
    if t == 9 { return __num_str(p) }
    if t == 2 {
        if p != 0 {
            return 'true'
        }
        return 'false'
    }
    if t == 3 {
        return __quote(v)
    }
    if t == 4 {
        return __cat(__cat('[', __join_shown(p + 8, __load(p))), ']')
    }
    if t == 5 {
        return __show_data(p)
    }
    if t == 6 {
        return '<fn>'
    }
    if t == 7 {
        return __mkv(3, p)
    }
    '<ref>'
}

fn __show_data(p: int) -> String {
    let ctor = __load(p)
    let n = __load(p + 8)
    let name = __ctor_name(ctor)
    let kind = __ctor_kind(ctor)
    if kind != 0 {
        let base = name
        // struct Name(型, ...) は Name(1, 2)
        if kind == 2 {
            return __cat(__cat(__cat(base, '('), __join_shown(p + 16, n)), ')')
        }
        let mut out = __cat(base, ' { ')
        let mut i = 0
        while i < n {
            if i > 0 {
                out = __cat(out, ', ')
            }
            out = __cat(__cat(__cat(out, __ctor_field(ctor, i)), ': '), __show(__getv(p + 16 + i * 16)))
            i = i + 1
        }
        return __cat(out, ' }')
    }
    if n == 0 {
        return name
    }
    __cat(__cat(__cat(name, '('), __join_shown(p + 16, n)), ')')
}

fn __to_str(v: int) -> String {
    let t = __tag(v)
    if t == 3 {
        return v
    }
    if t == 7 {
        return __mkv(3, __payload(v))
    }
    __show(v)
}

fn __write_str(fd: int, s: String) {
    let p = __payload(s)
    __sys_write(fd, p + 8, __load(p))
}

// 標準入力は呼び出し間で共有する区画 [読んだ位置][終わり][4096 バイト]（+144）に読み、改行までを返す。
// 区画が空の時だけ read を 1 回呼ぶので、対話で 1 行ずつ渡される入力も待たずに返せる
fn __input() -> String {
    let st = __static()
    let mut io = __load(st + 144)
    if io == 0 {
        io = __bump(4112)
        __store(io, 0)
        __store(io + 8, 0)
        __store(st + 144, io)
    }
    let mut capacity = 64
    let mut buffer = __alloc(capacity)
    let mut size = 0
    let mut done = false
    while !done {
        let mut pos = __load(io)
        let end = __load(io + 8)
        if pos == end {
            let read = __sys_read(0, io + 16, 4096)
            if read == 0 { done = true }
            else if read != 0 - 4 {
                if read < 0 { __die('標準入力を読めません') }
                __store(io, 0)
                __store(io + 8, read)
            }
        } else {
            let start = pos
            while pos < end && __load8(io + 16 + pos) != 10 { pos = pos + 1 }
            let n = pos - start
            if size + n > capacity {
                while size + n > capacity { capacity = capacity * 2 }
                let next = __alloc(capacity)
                __copy(next, buffer, size)
                buffer = next
            }
            __copy(buffer + size, io + 16 + start, n)
            size = size + n
            if pos < end {
                pos = pos + 1
                if size > 0 && __load8(buffer + size - 1) == 13 { size = size - 1 }
                done = true
            }
            __store(io, pos)
        }
    }
    __utf8_validate(buffer, size)
    __bytes(buffer, size)
}

// UTF-8 の過長表現・代理符号・範囲外・途中で終わる文字を拒否する
fn __utf8_validate(buffer: int, size: int) {
    if !__utf8_valid(buffer, size) { __die('標準入力の文字コードが UTF-8 でない') }
}
fn __utf8_valid(buffer: int, size: int) -> bool {
    let mut i = 0
    while i < size {
        let b = __load8(buffer + i)
        let mut n = 1
        let mut minimum = 0
        let mut codepoint = b
        if b >= 128 {
            if b >= 194 && b <= 223 { n = 2; minimum = 128; codepoint = b - 192 }
            else if b >= 224 && b <= 239 { n = 3; minimum = 2048; codepoint = b - 224 }
            else if b >= 240 && b <= 244 { n = 4; minimum = 65536; codepoint = b - 240 }
            else { return false }
            if i + n > size { return false }
            let mut j = 1
            while j < n {
                let c = __load8(buffer + i + j)
                if c < 128 || c > 191 { return false }
                codepoint = codepoint * 64 + c - 128
                j = j + 1
            }
            if codepoint < minimum || codepoint > 1114111 || (codepoint >= 55296 && codepoint <= 57343) { return false }
        }
        i = i + n
    }
    true
}

// 1 回の書き込みで 1 行を出す（JavaScript の接続コードは 1 回ごとに 1 行として渡す）
fn __put(v: int) {
    __write_str(1, __cat(__to_str(v), '\n'))
}

fn __panic(msg: String) {
    __die(msg)
}

// 文字列の組み込み

fn __len(v: int) -> int {
    if __tag(v) == 3 {
        return __utf8_len(__payload(v))
    }
    __load(__payload(v))
}

fn __starts_with(s: String, pre: String) -> bool {
    let ps = __payload(s)
    let pp = __payload(pre)
    let n = __load(pp)
    if __load(ps) < n {
        return false
    }
    __str_eq(pp, __payload(__bytes(ps + 8, n)))
}

// 区切りで分ける。空の区切りは 1 文字ずつ、前後に空の文字列を付ける（インタプリタと同じ）
fn __split(s: String, sep: String) -> Vec<String> {
    let ps = __payload(s)
    let pp = __payload(sep)
    let n = __load(ps)
    let m = __load(pp)
    let mut out = __arr_new(0)
    if m == 0 {
        out = __arr_push_own(out, '')
        let mut i = 0
        while i < n {
            let mut e = i + 1
            while e < n && __and(__load8(ps + 8 + e), 192) == 128 {
                e = e + 1
            }
            out = __arr_push_own(out, __bytes(ps + 8 + i, e - i))
            i = e
        }
        return __share(__mkv(4, __arr_push_own(out, '')))
    }
    let mut start = 0
    let mut i = 0
    while i + m <= n {
        if __str_eq(pp, __payload(__bytes(ps + 8 + i, m))) {
            out = __arr_push_own(out, __bytes(ps + 8 + start, i - start))
            i = i + m
            start = i
        } else {
            i = i + 1
        }
    }
    __share(__mkv(4, __arr_push_own(out, __bytes(ps + 8 + start, n - start))))
}

fn __chars(s: String) -> Vec<String> {
    let ps = __payload(s)
    let n = __load(ps)
    let mut out = __arr_new(0)
    let mut i = 0
    while i < n {
        let mut e = i + 1
        while e < n && __and(__load8(ps + 8 + e), 192) == 128 {
            e = e + 1
        }
        out = __arr_push_own(out, __bytes(ps + 8 + i, e - i))
        i = e
    }
    __share(__mkv(4, out))
}

fn __code(s: String) -> int {
    let p = __payload(s)
    let n = __load(p)
    if n == 0 {
        return 0 - 1
    }
    let b = __load8(p + 8)
    if b < 128 {
        return b
    }
    let mut k = 1
    if b >= 224 {
        k = 2
    }
    if b >= 240 {
        k = 3
    }
    let mut c = __and(b, __shr(63, k))
    let mut i = 1
    while i <= k && i < n {
        c = __or(__shl(c, 6), __and(__load8(p + 8 + i), 63))
        i = i + 1
    }
    c
}

fn __char_from_codepoint(c: int) -> String {
    if c < 0 || c > 1114111 || c >= 55296 && c <= 57343 {
        return ''
    }
    if c < 128 {
        return __byte(c)
    }
    let p = __str_new(4)
    let mut n = 4
    if c < 2048 {
        n = 2
        __store8(p + 8, __or(192, __shr(c, 6)))
        __store8(p + 9, __or(128, __and(c, 63)))
    } else {
        if c < 65536 {
            n = 3
            __store8(p + 8, __or(224, __shr(c, 12)))
            __store8(p + 9, __or(128, __and(__shr(c, 6), 63)))
            __store8(p + 10, __or(128, __and(c, 63)))
        } else {
            __store8(p + 8, __or(240, __shr(c, 18)))
            __store8(p + 9, __or(128, __and(__shr(c, 12), 63)))
            __store8(p + 10, __or(128, __and(__shr(c, 6), 63)))
            __store8(p + 11, __or(128, __and(c, 63)))
        }
    }
    __bytes(p + 8, n)
}

// 文字の位置 i..j の部分。範囲外は実行時エラー
fn __slice(s: String, i: int, j: int) -> String {
    let p = __payload(s)
    let n = __load(p)
    if !(0 <= i && i <= j && j <= __utf8_len(p)) {
        __die(__cat(__cat(__cat(__cat('slice の範囲 ', __int_str(i)), '..'), __int_str(j)), ' が文字列の外'))
    }
    let mut c = 0
    let mut a = 0
    let mut b = n
    let mut k = 0
    let mut done = false
    while !done && k <= n {
        if k == n || __and(__load8(p + 8 + k), 192) != 128 {
            if c == i {
                a = k
            }
            if c == j {
                b = k
                done = true
            }
            c = c + 1
        }
        k = k + 1
    }
    __bytes(p + 8 + a, b - a)
}

fn __now_ms() -> int {
    __now()
}

// Vec

// Vec の位置の直前に [容量][所有の印] の 16 バイトを置く。要素は位置 + 8 から並ぶ。
// 所有の印が 1 の Vec は 1 つの mut 変数だけが指していて、__push_own・__set_at_own がその場で書き換える。
// 変数の値を読んで他へ渡す時は、ir.rs が __share で印を消す
fn __arr_new(n: int) -> int {
    let p = __gc_kind(__alloc(24 + n * 16), 1) + 16
    __store(p - 16, n)
    __store(p - 8, 0)
    __store(p, n)
    p
}

// 要素を 1 つ足した新しい Vec の位置
fn __arr_push(p: int, x: int) -> int {
    let n = __load(p)
    let q = __arr_new(n + 1)
    __copy(q + 8, p + 8, n * 16)
    __setv(q + 8 + n * 16, x)
    q
}

// 所有している Vec なら容量の余りに足す。足りなければ容量を 2 倍にして写し、写した先を所有する
fn __arr_push_own(p: int, x: int) -> int {
    let n = __load(p)
    if __load(p - 8) == 1 && n < __load(p - 16) {
        __setv(p + 8 + n * 16, x)
        __store(p, n + 1)
        return p
    }
    let mut cap = n * 2
    if cap < 4 {
        cap = 4
    }
    let q = __gc_kind(__alloc(24 + cap * 16), 1) + 16
    __store(q - 16, cap)
    __store(q - 8, 1)
    __store(q, n + 1)
    __copy(q + 8, p + 8, n * 16)
    __setv(q + 8 + n * 16, x)
    q
}

fn __push(v: int, x: int) -> int {
    __mkv(4, __arr_push(__payload(v), x))
}

fn __push_own(v: int, x: int) -> int {
    __mkv(4, __arr_push_own(__payload(v), x))
}

// Vec の値なら所有の印を消す。それ以外の値はそのまま
fn __share(v: int) -> int {
    if __tag(v) == 4 {
        __store(__payload(v) - 8, 0)
    }
    v
}

fn __set_at(v: int, i: int, x: int) -> int {
    let p = __payload(v)
    let n = __load(p)
    let q = __arr_new(n)
    __copy(q + 8, p + 8, n * 16)
    __setv(q + 8 + i * 16, x)
    __mkv(4, q)
}

fn __set_at_own(v: int, i: int, x: int) -> int {
    let p = __payload(v)
    if __load(p - 8) == 1 {
        __setv(p + 8 + i * 16, x)
        return v
    }
    let w = __set_at(v, i, x)
    __store(__payload(w) - 8, 1)
    w
}

// 並行に動かす処理が無いので、順に呼ぶ
fn __all(fs: int) -> int {
    let p = __payload(fs)
    let n = __load(p)
    let q = __arr_new(n)
    // 利用者の関数を呼ぶ間、fs と作りかけの q を GC の根に置く。q の要素は 0（unit）で埋めておく
    __zero(q + 8, n * 16)
    let roots = __frame_push(2)
    __setv(roots, fs)
    __setv(roots + 16, __mkv(4, q))
    let mut i = 0
    while i < n {
        let f = __getv(p + 8 + i * 16)
        __setv(q + 8 + i * 16, f())
        if __aborting() {
            __store(__static() + 64, roots)
            return
        }
        i = i + 1
    }
    __store(__static() + 64, roots)
    __mkv(4, q)
}

// Ref

fn __new_ref(x: int) -> int {
    let p = __gc_kind(__alloc(16), 3)
    __setv(p, x)
    __mkv(8, p)
}

fn __read(r: int) -> int {
    __getv(__payload(r))
}

fn __write(r: int, x: int) {
    __setv(__payload(r), x)
}

// spawn。その場で最後まで実行し、Task<T> は結果の値そのものにする
// ponytail: 実行の待ち行列を持たないので、spawn した処理が JavaScript より先に走る。poll で進めるステートマシンに変換すれば並行に動かせる
fn __spawn(f: int) -> int {
    f()
}

fn __task_then(t: int, f: int) {
    f(t)
}

fn __task_done(t: int) -> bool {
    true
}

fn __task_wait(t: int) -> int {
    t
}

fn __task_value(t: int) -> int {
    t
}

// 構築子と関数の値

fn __data_new(ctor: int, n: int) -> int {
    let p = __gc_kind(__alloc(16 + n * 16), 2)
    __store(p, ctor)
    __store(p + 8, n)
    p
}

fn __fn_new(code: int, n: int) -> int {
    let p = __gc_kind(__alloc(16 + n * 16), 2)
    __store(p, code)
    __store(p + 8, n)
    p
}

// WASM の境界。文字列は (長さ << 32) | バイト列の位置 の 1 つの整数で渡す

fn __str_in(packed: int) -> String {
    __bytes(__and(packed, 4294967295), __shr(packed, 32))
}

fn __str_out(s: String) -> int {
    let p = __payload(s)
    __or(__shl(__load(p), 32), p + 8)
}

// 要求と提供（with）。with の本体を抜ける中断は、行き先の番号を置き、呼び出しの後の検査で 1 段ずつ戻って伝える（ir.rs の checked）

fn __aborting() -> bool {
    __load(__static() + 24) != 0
}

// with の番号を決める。腕を作る前に決め、腕の中の exit の行き先にする
fn __handle_id() -> int {
    let st = __static()
    let id = __load(st + 56) + 1
    __store(st + 56, id)
    id
}

// 処理中の with: [外側の with][番号][操作の名前の Vec][腕の Vec]
fn __handle_push(id: int, ops: int, arms: int) -> int {
    let st = __static()
    let h = __gc_kind(__alloc(48), 4)
    __store(h, __load(st + 48))
    __store(h + 8, id)
    __setv(h + 16, ops)
    __setv(h + 32, arms)
    __store(st + 48, h)
    id
}

// 本体を終えた with を外す。この with への中断なら、腕の値が with の値になる
fn __handle_end(id: int, v: int) -> int {
    let st = __static()
    __store(st + 48, __load(__load(st + 48)))
    if __load(st + 24) == id {
        __store(st + 24, 0)
        return __getv(st + 32)
    }
    v
}

// exit。番号 id の with へ、値 v で抜ける中断の印を置く
fn __exit(id: int, v: int) -> int {
    let st = __static()
    __store(st + 24, id)
    __setv(st + 32, v)
    0
}

// #resume した腕の値は構築子の番号 -1 の値で表す
fn __resume(v: int) -> int {
    let p = __data_new(0 - 1, 1)
    __setv(p + 16, v)
    __mkv(5, p)
}

// 要求の呼び出しを、内側の with から探して腕で処理する。腕の中の要求は、その with より外で処理する
fn __perform(name: String, args: int) -> int {
    let st = __static()
    let mut h = __load(st + 48)
    while h != 0 {
        let ops = __payload(__getv(h + 16))
        let n = __load(ops)
        let mut k = 0
        while k < n {
            if __str_eq(__payload(__getv(ops + 8 + k * 16)), __payload(name)) {
                let arm = __getv(__payload(__getv(h + 32)) + 8 + k * 16)
                let saved = __load(st + 48)
                __store(st + 48, __load(h))
                // 腕を動かす間、外した with の記録と引数を GC の根に置く
                let roots = __frame_push(2)
                __setv(roots, __mkv(10, saved))
                __setv(roots + 16, args)
                let r = arm(args)
                __store(st + 64, roots)
                __store(st + 48, saved)
                if __aborting() {
                    return
                }
                if __tag(r) == 5 && __load(__payload(r)) == 0 - 1 {
                    return __getv(__payload(r) + 16)
                }
                __store(st + 24, __load(h + 8))
                __setv(st + 32, r)
                return
            }
            k = k + 1
        }
        h = __load(h)
    }
    __die('要求を提供する with が無い')
}
