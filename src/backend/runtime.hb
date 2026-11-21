// IR のランタイム。WASM とネイティブの両方で同じものを使う。
// 型検査は通さず、構文解析だけして IR に変換する。演算子はすべて値の中身（64 ビット整数）の演算になる。
// __load などの __ で始まる組み込みは、IR の命令に直接変換する（src/backend/ir.rs の prim）。
//
// 値の型の番号: 0 unit, 1 int, 2 bool, 3 String, 4 Vec, 5 type の値・構造体, 6 関数, 7 型, 8 Ref, 9 number（中身は f64 のビット列）
// 10 は with の記録の並び（ランタイムの中だけで使う）、11 は Promise（非同期。下の __promise_new）
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
// GC の間に生きていない。ランタイムの関数から利用者の関数を呼ぶ所（__all など）は、持っている値をシャドースタックに置く

// ランタイムの状態は __static() から 200 バイト:
//   +0 確保の次の位置、+8 区画の終わり、+16 実行時エラーの文字列（WASM だけ使う）
//   +24 中断の行き先（with の番号。0 は中断していない、-1 は実行時エラー）、+32 中断で運ぶ値
//   +48 処理中のいちばん内側の with、+56 最後に付けた with の番号
//   +64 シャドースタックの先頭、+72 その始まり、+80 その終わり
//   +88 確保した区画の並び、+96 前回の GC の後に確保したバイト数、+104 GC を始めるバイト数
//   +112 これより前の位置は文字列定数（WASM は接続コードが書く。0 なら __static() の位置）
//   +120 空き区画の並びの表（1024 バイトまでは 16 バイトごと、それより大きい区画は 2 のべきごとの 128 本）
//   +128 印を付ける作業の並び、+136 その容量
//   +144 標準入力の区画（__input）
//   +152 非同期の仕事の並びの先頭（値）、+168 その末尾（__enqueue）
//   +176 実行の入口、コルーチン、then の関数で実行時エラーを中断として運ぶ間は 1
//   +184 then の関数で最初に起きた実行時エラー（文字列の値）
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
    if tag < 3 || tag == 9 || tag > 11 {
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
    __gc_mark(__getv(st + 152))
    __gc_mark(__getv(st + 184))
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

// 非同期処理または try_overflow の内側では、整数の範囲超過を中断番号 -2 で伝える。
fn __integer_overflow() {
    let st = __static()
    let mut h = __load(st + 48)
    while h != 0 && __tag(__overflow_arm(h)) != 6 {
        h = __load(h)
    }
    if h != 0 || __load(st + 176) != 0 {
        __exit(0 - 2, 'オーバーフロー')
        return
    }
    __overflow()
}

fn __div_zero() {
    __die('0 で除算した')
}

// 利用者の整数演算。ランタイム内部の64ビット演算とは分ける。
fn __i53(n: int) -> int {
    if n > 9007199254740991 || n < -9007199254740991 {
        __integer_overflow()
        return 0
    }
    n
}
fn __iadd(a: int, b: int) -> int { __i53(a + b) }
fn __isub(a: int, b: int) -> int { __i53(a - b) }
fn __imul(a: int, b: int) -> int {
    // 安全整数の積は64ビットを超えることがあるので、掛ける前に範囲を調べる
    let x = if a < 0 { 0 - a } else { a }
    let y = if b < 0 { 0 - b } else { b }
    if y != 0 && x > 9007199254740991 / y {
        __integer_overflow()
        return 0
    }
    a * b
}
fn __ineg(a: int) -> int { 0 - a }
fn __idiv(a: int, b: int) -> int {
    if b == 0 {
        __panic('0 で除算した')
        return 0
    }
    a / b
}
fn __irem(a: int, b: int) -> int {
    if b == 0 {
        __panic('0 で除算した')
        return 0
    }
    a % b
}

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
        __panic('負の指数')
        return 0
    }
    let mut n = b
    if n > 52 {
        if a > 1 || a < 0 - 1 {
            __integer_overflow()
            return 0
        }
        n = 52 + n % 2
    }
    let mut r = 1
    while n > 0 {
        r = __imul(r, a)
        if __aborting() { return 0 }
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
    if __tag(a) == 9 || __tag(b) == 9 { return __num_result(__f_add(__num(a), __num(b))) }
    if __tag(a) == 1 { return __iadd(a, b) }
    __cat(a, b)
}

fn __num_finite(bits: int) -> bool {
    __and(__shr(__payload(bits), 52), 2047) != 2047
}

fn __num_checked(bits: int) -> number {
    if !__num_finite(bits) { __die('number の演算結果が有限でない') }
    __mkv(9, __payload(bits))
}

fn __num_result(bits: int) -> number {
    if !__num_finite(bits) {
        __panic('number の演算結果が有限でない')
        return 0.0
    }
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
    if __tag(a) == 9 || __tag(b) == 9 { return __num_result(__f_sub(__num(a), __num(b))) }
    __isub(a, b)
}
fn __mul(a: int, b: int) -> int {
    if __tag(a) == 9 || __tag(b) == 9 { return __num_result(__f_mul(__num(a), __num(b))) }
    __imul(a, b)
}
fn __div(a: int, b: int) -> int {
    if __tag(a) == 9 || __tag(b) == 9 {
        if __f_eq(__num(b), 0.0) {
            __panic('0 で除算した')
            return 0.0
        }
        return __num_result(__f_div(__num(a), __num(b)))
    }
    __idiv(a, b)
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
    if __f_lt(n, -9007199254740991.0) || __f_lt(9007199254740991.0, n) {
        __panic('floor の結果が int の範囲外')
        return 0
    }
    __f_to_int(n)
}
fn __num_sqrt(value: number) -> number {
    let v = __num(value)
    if __f_lt(v, 0.0) {
        __panic('sqrt の引数が負')
        return 0.0
    }
    __mkv(9, __payload(__f_sqrt(v)))
}

fn __value_pow(left: int, right: int) -> int {
    if __tag(left) != 9 && __tag(right) != 9 { return __pow(left, right) }
    let a = __num(left)
    let b = __num(right)
    if !__f_eq(__f_floor(b), b) {
        __panic('** の指数は整数値に限る')
        return 0.0
    }
    let mut n = b
    let mut factor = a
    if __f_lt(n, 0.0) {
        n = __f_neg(n)
        factor = __f_div(1.0, a)
    }
    let mut result = 1.0
    while !__f_eq(n, 0.0) {
        let half = __f_floor(__f_div(n, 2.0))
        if !__f_eq(__f_mul(half, 2.0), n) { result = __f_mul(result, factor) }
        n = half
        if !__f_eq(n, 0.0) { factor = __f_mul(factor, factor) }
    }
    __num_result(result)
}

// 仮数の整数除算で余りを求め、除算の結果が桁あふれする場合も扱う
fn __rem(left: int, right: int) -> int {
    if __tag(left) != 9 && __tag(right) != 9 { return __irem(left, right) }
    let a = __num(left)
    let b = __num(right)
    let sign = __shl(__shr(__payload(a), 63), 63)
    let mut x = __and(__payload(a), 9223372036854775807)
    let y = __and(__payload(b), 9223372036854775807)
    if y == 0 {
        __panic('0 で除算した')
        return 0.0
    }
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
        let ctor = __load(pa)
        // Task は同じアドレスの場合だけ等しい。
        if ctor == 0 - 2 || ctor != __load(pb) {
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

// dst が 0 なら長さを数え、それ以外なら引用符の内側を dst に書く
fn __quote_contents(src: int, len: int, dst: int) -> int {
    let mut size = 0
    let mut i = 0
    while i < len {
        let mut c = __load8(src + i)
        let mut width = 1
        // U+0080〜U+009F の UTF-8 は C2 80〜C2 9F
        if c == 194 && i + 1 < len {
            let next = __load8(src + i + 1)
            if next >= 128 && next <= 159 {
                c = next
                width = 2
            }
        }
        let escaped = if c == 34 { '\\"' }
        else if c == 92 { '\\\\' }
        else if c == 8 { '\\b' }
        else if c == 9 { '\\t' }
        else if c == 10 { '\\n' }
        else if c == 12 { '\\f' }
        else if c == 13 { '\\r' }
        else { '' }
        let bytes = if __load(__payload(escaped)) != 0 { 2 }
        else if c < 32 || c == 127 || width == 2 { 6 }
        else { 1 }
        if dst != 0 {
            let at = dst + size
            if bytes == 1 {
                __store8(at, c)
            } else if bytes == 2 {
                __copy(at, __payload(escaped) + 8, 2)
            } else {
                let hex = __payload('0123456789abcdef') + 8
                __copy(at, __payload('\\u00') + 8, 4)
                __store8(at + 4, __load8(hex + c / 16))
                __store8(at + 5, __load8(hex + c % 16))
            }
        }
        size = size + bytes
        i = i + width
    }
    size
}

fn __quote(s: String) -> String {
    let src = __payload(s)
    let len = __load(src)
    let size = __quote_contents(src + 8, len, 0)
    let out = __alloc(10 + size)
    __store(out, size + 2)
    __store8(out + 8, 34)
    __quote_contents(src + 8, len, out + 9)
    __store8(out + 9 + size, 34)
    __mkv(3, out)
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
    if ctor == 0 - 2 { return '<task>' }
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
                if read < 0 {
                    __panic('標準入力を読めません')
                    return ''
                }
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
    if !__utf8_valid(buffer, size) {
        __panic('標準入力の文字コードが UTF-8 でない')
        return ''
    }
    __bytes(buffer, size)
}

// UTF-8 の過長表現・代理符号・範囲外・途中で終わる文字を拒否する
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
    if __load(__static() + 176) != 0 {
        __exit(0 - 1, msg)
        return
    }
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
        __panic(__cat(__cat(__cat(__cat('slice の範囲 ', __int_str(i)), '..'), __int_str(j)), ' が文字列の外'))
        return ''
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

// 非同期（Async）。JavaScript の Promise とマイクロタスクと同じ順で動かす（js.rs の async 関数・await・$all・$spawn・$then）。
// Promise は型の番号 11 の値で、構造体と同じ形 [状態][3][値][反応の先頭][反応の末尾]。状態は 0 未完了・1 値・2 失敗。
// 反応と仕事は構造体 [種類][6][次][a][b][c][結果の状態][結果の値]。Promise が決まると反応を仕事の並び（+152 から）の末尾に移す。
// 失敗の値は中断 [-3][2][行き先の番号][値]。実行時エラーは行き先 -1 とメッセージを持つ。
// 仕事の種類: 1 コルーチンの再開、2 thenable（Promise で解決した Promise）、3 a を同じ結果で解決、4 all の 1 要素、
// 5 $all の await の続き（with の並びを b に戻して a を解決）、6 spawn の終わり、7 then
// コルーチンの区画は関数の値 [関数][数][Promise][状態の番号][再開の状態][再開の値][局所変数...]（ir.rs の Ctx::coroutine）

fn __promise_new() -> int {
    __mkv(11, __data_new(0, 3))
}

fn __job_new(kind: int, a: int, b: int, c: int) -> int {
    let p = __data_new(kind, 6)
    __zero(p + 16, 96)
    __setv(p + 32, a)
    __setv(p + 48, b)
    __setv(p + 64, c)
    p
}

// 仕事の並びの末尾に足す
fn __enqueue(node: int) {
    let st = __static()
    __setv(node + 16, __mkv(0, 0))
    let tail = __load(st + 168)
    if tail == 0 {
        __setv(st + 152, __mkv(5, node))
    } else {
        __setv(tail + 16, __mkv(5, node))
    }
    __store(st + 168, node)
}

// 結果の決まった反応を仕事にする
fn __enqueue_with(node: int, status: int, v: int) {
    __setv(node + 80, __mkv(1, status))
    __setv(node + 96, v)
    __enqueue(node)
}

// Promise を状態 status と値 v で決める。決まっていれば何もしない
fn __settle(pv: int, status: int, v: int) {
    let p = __payload(pv)
    if __load(p) != 0 {
        return
    }
    __store(p, status)
    __setv(p + 16, v)
    let mut r = __getv(p + 32)
    __setv(p + 32, __mkv(0, 0))
    __setv(p + 48, __mkv(0, 0))
    while __tag(r) == 5 {
        let node = __payload(r)
        r = __getv(node + 16)
        __enqueue_with(node, status, v)
    }
}

// resolve 関数。値が Promise なら、その Promise の then を仕事にする（NewPromiseResolveThenableJob）
fn __resolve(pv: int, v: int) {
    if __tag(v) == 11 {
        __enqueue(__job_new(2, pv, v, __mkv(0, 0)))
        return
    }
    __settle(pv, 1, v)
}

// PromiseResolve(v).then(反応)。決まっていれば仕事にし、未完了なら反応の末尾に足す
fn __react(v: int, node: int) {
    if __tag(v) != 11 {
        __enqueue_with(node, 1, v)
        return
    }
    let p = __payload(v)
    if __load(p) != 0 {
        __enqueue_with(node, __load(p), __getv(p + 16))
        return
    }
    __setv(node + 16, __mkv(0, 0))
    if __tag(__getv(p + 32)) == 5 {
        __setv(__payload(__getv(p + 48)) + 16, __mkv(5, node))
    } else {
        __setv(p + 32, __mkv(5, node))
    }
    __setv(p + 48, __mkv(5, node))
}

// 中断中なら、その中断を失敗の値にして消す
fn __abort_info() -> int {
    let st = __static()
    let info = __data_new(0 - 3, 2)
    __setv(info + 16, __mkv(1, __load(st + 24)))
    __setv(info + 32, __getv(st + 32))
    __store(st + 24, 0)
    __mkv(5, info)
}

// 失敗の値の中断を置き直す
fn __abort_again(info: int) {
    let st = __static()
    let p = __payload(info)
    __store(st + 24, __payload(__getv(p + 16)))
    __setv(st + 32, __getv(p + 32))
}

fn __is_panic(info: int) -> bool {
    __payload(__getv(__payload(info) + 16)) < 0
}

fn __panic_message(info: int) -> String {
    __getv(__payload(info) + 32)
}

// コルーチンの区画。局所変数 n 個の分を含む
fn __co_new(code: int, n: int) -> int {
    let p = __fn_new(code, 4 + n)
    __zero(p + 16, (4 + n) * 16)
    __setv(p + 16, __promise_new())
    __setv(p + 32, __mkv(1, 0))
    __mkv(6, p)
}

fn __co_promise(frame: int) -> int {
    __getv(__payload(frame) + 16)
}

// 最初の実行と再開は同じ入口を通し、その間の実行時エラーを失敗として運ぶ
fn __co_step(frame: int) {
    let at = __static() + 176
    let saved = __load(at)
    __store(at, 1)
    frame()
    __store(at, saved)
}

// await。v が決まったら、コルーチンを再開する仕事を足す
fn __await(frame: int, v: int) {
    __react(v, __job_new(1, frame, __mkv(0, 0), __mkv(0, 0)))
}

// 再開した所で待った値を受け取る。失敗なら中断を置き直す（呼んだ後の中断の検査で抜ける）
fn __co_resume(frame: int) -> int {
    let p = __payload(frame)
    if __payload(__getv(p + 48)) == 2 {
        __abort_again(__getv(p + 64))
        return __mkv(0, 0)
    }
    __getv(p + 64)
}

// async 関数から戻る。中断中なら失敗にする
fn __co_return(frame: int, v: int) {
    let pv = __co_promise(frame)
    if __aborting() {
        __settle(pv, 2, __abort_info())
        return
    }
    __resolve(pv, v)
}

// 仕事を 1 つ動かす。仕事が無ければ false
fn __run_job() -> bool {
    let st = __static()
    let head = __getv(st + 152)
    if __tag(head) != 5 {
        return false
    }
    let node = __payload(head)
    let next = __getv(node + 16)
    __setv(st + 152, next)
    if __tag(next) != 5 {
        __store(st + 168, 0)
    }
    let kind = __load(node)
    let a = __getv(node + 32)
    let b = __getv(node + 48)
    let c = __getv(node + 64)
    let status = __payload(__getv(node + 80))
    let v = __getv(node + 96)
    if kind == 1 {
        let p = __payload(a)
        __setv(p + 48, __mkv(1, status))
        __setv(p + 64, v)
        __co_step(a)
    } else if kind == 2 {
        __react(b, __job_new(3, a, __mkv(0, 0), __mkv(0, 0)))
    } else if kind == 3 {
        if status == 1 {
            __resolve(a, v)
        } else {
            __settle(a, status, v)
        }
    } else if kind == 4 {
        if status == 2 {
            __settle(a, 2, v)
        } else {
            let state = __payload(b)
            __setv(__payload(__getv(state + 32)) + 8 + __payload(c) * 16, v)
            let left = __payload(__getv(state + 16)) - 1
            __setv(state + 16, __mkv(1, left))
            if left == 0 {
                __resolve(a, __getv(state + 32))
            }
        }
    } else if kind == 5 {
        __store(st + 48, __payload(b))
        if status == 1 {
            __resolve(a, v)
        } else {
            __settle(a, status, v)
        }
    } else if kind == 6 {
        let task = __payload(a)
        __setv(task + 16, __mkv(1, status))
        __setv(task + 32, v)
        if status == 2 {
            let message = if __is_panic(v) { __panic_message(v) } else { 'spawn した処理が with を抜けずに中断した' }
            __write_str(2, __cat(__cat('実行時エラー: ', message), '\n'))
        }
        __resolve(b, __mkv(0, 0))
    } else if kind == 7 {
        let task = __payload(a)
        if __payload(__getv(task + 16)) == 1 {
            let saved = __load(st + 176)
            __store(st + 176, 1)
            b(__getv(task + 32), __mkv(0, 0))
            __store(st + 176, saved)
            if __aborting() {
                let info = __abort_info()
                if __tag(__getv(st + 184)) == 0 {
                    let message = if __is_panic(info) { __panic_message(info) } else { 'with で処理されなかった中断' }
                    __setv(st + 184, message)
                }
            }
        }
    }
    true
}

fn __begin_entry() {
    __store(__static() + 176, 1)
}

// 仕事をすべて動かしてから入口の結果を返す。仕事の中で GC が動くので結果と中断を根に置く
fn __finish_entry(v: int) -> int {
    let st = __static()
    __store(st + 176, 0)
    let roots = __frame_push(2)
    __setv(roots, v)
    let failure = if __aborting() { __abort_info() } else { __mkv(0, 0) }
    __setv(roots + 16, failure)
    while __run_job() {
    }
    __store(st + 64, roots)
    if __tag(failure) != 0 {
        if __is_panic(failure) { __die(__panic_message(failure)) }
        __die('with で処理されなかった中断')
    }
    let mut result = v
    if __tag(v) == 11 {
        let p = __payload(v)
        let status = __load(p)
        if status == 0 { __die('非同期処理が完了しなかった') }
        if status == 2 {
            let info = __getv(p + 16)
            if __is_panic(info) { __die(__panic_message(info)) }
            __die('非同期処理が中断した')
        }
        result = __getv(p + 16)
    }
    let callback_error = __getv(st + 184)
    if __tag(callback_error) == 3 { __die(callback_error) }
    result
}

// all。$all と同じく、すべての関数を呼んでから Promise.all にし、それを待つ async 関数の Promise を返す
// c は all を呼んだ所の文脈。Async の値（引数なしの無名関数）は最後の引数に文脈を受け取る
fn __all(fs: int, c: int) -> int {
    let st = __static()
    let h = __load(st + 48)
    let p = __payload(fs)
    let n = __load(p)
    let outer = __promise_new()
    let ps = __arr_new(n)
    __zero(ps + 8, n * 16)
    // 利用者の関数を呼ぶ間、fs と結果の並びと文脈を GC の根に置く
    let roots = __frame_push(3)
    __setv(roots, fs)
    __setv(roots + 16, __mkv(4, ps))
    __setv(roots + 32, c)
    let mut i = 0
    while i < n {
        let f = __getv(p + 8 + i * 16)
        __store(st + 48, h)
        __setv(ps + 8 + i * 16, f(c))
        i = i + 1
    }
    __store(st + 64, roots)
    let all = __promise_new()
    let results = __arr_new(n)
    __zero(results + 8, n * 16)
    let state = __data_new(0, 2)
    __setv(state + 16, __mkv(1, n))
    __setv(state + 32, __mkv(4, results))
    if n == 0 {
        __resolve(all, __mkv(4, results))
    }
    i = 0
    while i < n {
        __react(__getv(ps + 8 + i * 16), __job_new(4, all, __mkv(5, state), __mkv(1, i)))
        i = i + 1
    }
    __react(all, __job_new(5, outer, __mkv(10, h), __mkv(0, 0)))
    outer
}

// spawn。Async<T> の値を走らせ始め、Task [-2][3][状態][値][then を付ける Promise] を返す
fn __spawn(f: int) -> int {
    let st = __static()
    let h = __load(st + 48)
    let p = f(__mkv(0, 0))
    let task = __data_new(0 - 2, 3)
    __setv(task + 16, __mkv(1, 0))
    __setv(task + 32, __mkv(0, 0))
    let next = __promise_new()
    __setv(task + 48, next)
    __react(p, __job_new(6, __mkv(5, task), next, __mkv(0, 0)))
    __store(st + 48, h)
    __mkv(5, task)
}

fn __task_then(t: int, f: int) {
    __react(__getv(__payload(t) + 48), __job_new(7, t, f, __mkv(0, 0)))
}

fn __task_done(t: int) -> bool {
    __payload(__getv(__payload(t) + 16)) == 1
}

fn __task_value(t: int) -> int {
    if __task_done(t) { return __getv(__payload(t) + 32) }
    __mkv(0, 0)
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

// Vec と構造体の境界。値を 8 バイトの語の並びにして (長さ << 32) | 位置 で渡す。
// 値は [型の番号][中身]、unit は [0]、文字列は [3][長さ][バイト列（8 バイトに切り上げ）]、
// Vec は [4][要素数][要素...]、構造体は [5][構築子の番号][フィールド数][フィールド...]

fn __val_size(v: int) -> int {
    let t = __tag(v)
    if t == 0 {
        return 8
    }
    if t == 3 {
        return 16 + (__load(__payload(v)) + 7) / 8 * 8
    }
    if t == 4 || t == 5 {
        let p = __payload(v)
        let mut at = p + 8
        let mut n = __load(p)
        let mut s = 16
        if t == 5 {
            at = p + 16
            n = __load(p + 8)
            s = 24
        }
        let mut i = 0
        while i < n {
            s = s + __val_size(__getv(at + i * 16))
            i = i + 1
        }
        return s
    }
    16
}

// v を q から書き、書き終えた次の位置を返す
fn __val_write(v: int, q: int) -> int {
    let t = __tag(v)
    __store(q, t)
    if t == 0 {
        return q + 8
    }
    let p = __payload(v)
    if t == 3 {
        let n = __load(p)
        __store(q + 8, n)
        __copy(q + 16, p + 8, n)
        return q + 16 + (n + 7) / 8 * 8
    }
    if t == 4 || t == 5 {
        let mut at = p + 8
        let mut n = __load(p)
        let mut r = q + 16
        if t == 5 {
            __store(q + 8, __load(p))
            at = p + 16
            n = __load(p + 8)
            __store(q + 16, n)
            r = q + 24
        } else {
            __store(q + 8, n)
        }
        let mut i = 0
        while i < n {
            r = __val_write(__getv(at + i * 16), r)
            i = i + 1
        }
        return r
    }
    __store(q + 8, p)
    q + 16
}

fn __val_out(v: int) -> int {
    let n = __val_size(v)
    let q = __alloc(n)
    __val_write(v, q)
    __or(__shl(n, 32), q)
}

fn __val_in(packed: int) -> int {
    let cursor = __alloc(8)
    __store(cursor, __and(packed, 4294967295))
    __val_read(cursor)
}

// cursor が指す位置から値を 1 つ読み、cursor を次の位置へ進める
fn __val_read(cursor: int) -> int {
    let q = __load(cursor)
    let t = __load(q)
    if t == 0 {
        __store(cursor, q + 8)
        return __mkv(0, 0)
    }
    if t == 3 {
        let n = __load(q + 8)
        __store(cursor, q + 16 + (n + 7) / 8 * 8)
        return __bytes(q + 16, n)
    }
    if t == 4 {
        let n = __load(q + 8)
        __store(cursor, q + 16)
        let p = __arr_new(n)
        let mut i = 0
        while i < n {
            __setv(p + 8 + i * 16, __val_read(cursor))
            i = i + 1
        }
        return __mkv(4, p)
    }
    if t == 5 {
        let n = __load(q + 16)
        __store(cursor, q + 24)
        let p = __data_new(__load(q + 8), n)
        let mut i = 0
        while i < n {
            __setv(p + 16 + i * 16, __val_read(cursor))
            i = i + 1
        }
        return __mkv(5, p)
    }
    __store(cursor, q + 16)
    __mkv(t, __load(q + 8))
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

fn __overflow_arm(h: int) -> int {
    let ops = __payload(__getv(h + 16))
    let n = __load(ops)
    let mut i = 0
    while i < n {
        let op = __getv(ops + 8 + i * 16)
        if __str_eq(__payload(op), __payload('std::IntOverflow::raise_overflow')) {
            return __getv(__payload(__getv(h + 32)) + 8 + i * 16)
        }
        i = i + 1
    }
    __mkv(0, 0)
}

// 本体を終えた with を外す。この with への中断なら、腕の値が with の値になる
fn __handle_end(id: int, v: int) -> int {
    let st = __static()
    let h = __load(st + 48)
    __store(st + 48, __load(h))
    if __load(st + 24) == 0 - 2 {
        let arm = __overflow_arm(h)
        if __tag(arm) == 6 {
            __store(st + 24, 0)
            arm(__mkv(0, 0))
        }
    }
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

// 文脈（要求の提供）の組 [-4][3][操作の名前][提供した関数][外の文脈]。check/context.rs が #ctx_put・#ctx_get で作り、引く
fn __ctx_put(c: int, k: String, f: int) -> int {
    let p = __data_new(0 - 4, 3)
    __setv(p + 16, k)
    __setv(p + 32, f)
    __setv(p + 48, c)
    __mkv(5, p)
}

// 内側の組から探す。型検査が提供を確かめているので、見つからないことは無い
fn __ctx_get(c: int, k: String) -> int {
    let mut p = __payload(c)
    while !__str_eq(__payload(__getv(p + 16)), __payload(k)) {
        p = __payload(__getv(p + 48))
    }
    __getv(p + 32)
}
