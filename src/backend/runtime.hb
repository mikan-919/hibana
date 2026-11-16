// IR のランタイム。WASM とネイティブの両方で同じものを使う。
// 型検査は通さず、構文解析だけして IR に変換する。演算子はすべて値の中身（64 ビット整数）の演算になる。
// __load などの __ で始まる組み込みは、IR の命令に直接変換する（src/backend/ir.rs の prim）。
//
// 値の型の番号: 0 unit, 1 int, 2 bool, 3 String, 4 Vec, 5 type の値・構造体, 6 関数, 7 型, 8 Ref, 9 num（中身は f64 のビット列）
// ヒープ上の形（各欄は 8 バイト、値は型の番号と中身の 16 バイト）:
//   String: [長さ][UTF-8 のバイト列]
//   Vec: [要素数][値...]
//   type の値・構造体: [構築子の番号][フィールド数][値...]
//   関数: [関数の番号][写し取った変数の数][値...]
//   Ref: [値]
//   型: 中身は型の名前の String
//
// ponytail: 確保した領域を回収しない（GC が無い）。長く動くプログラムでは、ここにシャドースタックを使う GC を足す

// ランタイムの状態は __static() から 64 バイト:
//   +0 確保の次の位置、+8 区画の終わり、+16 実行時エラーの文字列（WASM だけ使う）
//   +24 中断の行き先（handle の番号。0 は中断していない）、+32 中断で handle に渡す値
//   +48 処理中のいちばん内側の handle、+56 最後に付けた handle の番号
fn __alloc(n: int) -> int {
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

fn __die(msg: String) {
    __fail(msg)
}

fn __overflow() {
    __die('オーバーフロー')
}

fn __div_zero() {
    __die('0 で除算した')
}

// 指数が 64 を超えると、-1・0・1 以外は範囲外になる。-1・0・1 は偶奇だけで決まる
fn __pow(a: int, b: int) -> int {
    if b < 0 {
        __die('負の指数')
    }
    let mut n = b
    if n > 64 {
        if a > 1 || a < 0 - 1 {
            __overflow()
        }
        n = 64 + n % 2
    }
    let mut r = 1
    while n > 0 {
        r = r * a
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

// + の整数以外。文字列の連結
fn __add(a: String, b: String) -> String {
    if __tag(a) == 9 { return __num_checked(__f_add(a, b)) }
    __cat(a, b)
}

fn __num_checked(bits: int) -> num {
    if __and(__shr(__payload(bits), 52), 2047) == 2047 { __die('num の演算結果が有限でない') }
    __mkv(9, __payload(bits))
}

fn __num(v: int) -> num {
    if __tag(v) == 9 { return __num_checked(v) }
    __num_checked(__f_from_int(v))
}

fn __neg(v: int) -> int {
    if __tag(v) == 9 { return __num_checked(__f_neg(v)) }
    0 - v
}
fn __sub(a: int, b: int) -> int {
    if __tag(a) == 9 { return __num_checked(__f_sub(a, b)) }
    a - b
}
fn __mul(a: int, b: int) -> int {
    if __tag(a) == 9 { return __num_checked(__f_mul(a, b)) }
    a * b
}
fn __div(a: int, b: int) -> int {
    if __tag(a) == 9 {
        if __f_eq(b, 0.0) { __div_zero() }
        return __num_checked(__f_div(a, b))
    }
    a / b
}
fn __lt(a: int, b: int) -> bool {
    if __tag(a) == 9 { return __f_lt(a, b) }
    a < b
}
fn __le(a: int, b: int) -> bool {
    if __tag(a) == 9 { return __f_le(a, b) }
    a <= b
}
fn __gt(a: int, b: int) -> bool { __lt(b, a) }
fn __ge(a: int, b: int) -> bool { __le(b, a) }

fn __num_floor(v: num) -> int {
    let n = __f_floor(v)
    if __f_lt(n, -9223372036854775808.0) || !__f_lt(n, 9223372036854775808.0) { __die('floor の結果が int の範囲外') }
    __f_to_int(n)
}
fn __num_sqrt(v: num) -> num {
    if __f_lt(v, 0.0) { __die('sqrt の引数が負') }
    __num_checked(__f_sqrt(v))
}

// ponytail: num の指数は int の範囲の整数のみ。小数の指数には対数・指数関数の実装が必要
fn __value_pow(a: int, b: int) -> int {
    if __tag(a) != 9 { return __pow(a, b) }
    let mut n = __num_floor(b)
    if !__f_eq(__f_from_int(n), b) { __die('num の小数の指数は未対応') }
    let mut factor = a
    if n < 0 {
        if __f_eq(a, 0.0) { __die('num の演算結果が有限でない') }
        factor = __num_checked(__f_div(1.0, a))
    }
    let mut result = 1.0
    while n != 0 {
        if n % 2 != 0 { result = __num_checked(__f_mul(result, factor)) }
        n = n / 2
        if n != 0 { factor = __num_checked(__f_mul(factor, factor)) }
    }
    result
}

// 仮数の整数除算で余りを求め、除算の結果が桁あふれする場合も扱う
fn __rem(a: int, b: int) -> int {
    if __tag(a) != 9 { return a % b }
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
        if n >= 40 { __die('num の表示用整数が範囲外') }
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
            if count >= 17 { __die('num の表示を丸められない') }
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
    let np = __payload(name)
    let len = __load(np)
    let braces = len >= 2 && __load8(np + 8 + len - 2) == 123 && __load8(np + 8 + len - 1) == 125
    if braces {
        let base = __bytes(np + 8, len - 2)
        // struct Name(型, ...) は Name(1, 2)
        if n > 0 && __str_eq(__payload(__ctor_field(ctor, 0)), __payload('0')) {
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

fn __input() -> String {
    let mut capacity = 64
    let mut buffer = __alloc(capacity)
    let byte = __alloc(1)
    let mut size = 0
    let mut done = false
    while !done {
        // ponytail: 1バイトずつ読む。読み込みが律速になる場合は、呼び出し間で残りを保持するバッファを足す
        let read = __sys_read(0, byte, 1)
        if read == 0 { done = true }
        else if read != 0 - 4 {
            if read != 1 { __die('標準入力を読めません') }
            let b = __load8(byte)
            if b == 10 {
                if size > 0 && __load8(buffer + size - 1) == 13 { size = size - 1 }
                done = true
            } else {
                if size == capacity {
                    capacity = capacity * 2
                    let next = __alloc(capacity)
                    __copy(next, buffer, size)
                    buffer = next
                }
                __store8(buffer + size, b)
                size = size + 1
            }
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
        out = __arr_push(out, '')
        let mut i = 0
        while i < n {
            let mut e = i + 1
            while e < n && __and(__load8(ps + 8 + e), 192) == 128 {
                e = e + 1
            }
            out = __arr_push(out, __bytes(ps + 8 + i, e - i))
            i = e
        }
        return __mkv(4, __arr_push(out, ''))
    }
    let mut start = 0
    let mut i = 0
    while i + m <= n {
        if __str_eq(pp, __payload(__bytes(ps + 8 + i, m))) {
            out = __arr_push(out, __bytes(ps + 8 + start, i - start))
            i = i + m
            start = i
        } else {
            i = i + 1
        }
    }
    __mkv(4, __arr_push(out, __bytes(ps + 8 + start, n - start)))
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
        out = __arr_push(out, __bytes(ps + 8 + i, e - i))
        i = e
    }
    __mkv(4, out)
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

fn __arr_new(n: int) -> int {
    let p = __alloc(8 + n * 16)
    __store(p, n)
    p
}

// 要素を 1 つ足した新しい Vec の位置
// ponytail: 毎回すべて写す。ループで足すと要素数の 2 乗の時間。C 版の own（自分だけが指す配列はその場で足す）を移せば線形になる
fn __arr_push(p: int, x: int) -> int {
    let n = __load(p)
    let q = __arr_new(n + 1)
    __copy(q + 8, p + 8, n * 16)
    __setv(q + 8 + n * 16, x)
    q
}

fn __push(v: int, x: int) -> int {
    __mkv(4, __arr_push(__payload(v), x))
}

fn __set_at(v: int, i: int, x: int) -> int {
    let p = __payload(v)
    let n = __load(p)
    let q = __arr_new(n)
    __copy(q + 8, p + 8, n * 16)
    __setv(q + 8 + i * 16, x)
    __mkv(4, q)
}

// 並行に動かす処理が無いので、順に呼ぶ
fn __all(fs: int) -> int {
    let p = __payload(fs)
    let n = __load(p)
    let q = __arr_new(n)
    let mut i = 0
    while i < n {
        let f = __getv(p + 8 + i * 16)
        __setv(q + 8 + i * 16, f())
        if __aborting() {
            return
        }
        i = i + 1
    }
    __mkv(4, q)
}

// Ref

fn __new_ref(x: int) -> int {
    let p = __alloc(16)
    __setv(p, x)
    __mkv(8, p)
}

fn __read(r: int) -> int {
    __getv(__payload(r))
}

fn __write(r: int, x: int) {
    __setv(__payload(r), x)
}

// 構築子と関数の値

fn __data_new(ctor: int, n: int) -> int {
    let p = __alloc(16 + n * 16)
    __store(p, ctor)
    __store(p + 8, n)
    p
}

fn __fn_new(code: int, n: int) -> int {
    let p = __alloc(16 + n * 16)
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

// 効果。handle の本体を抜ける中断は、行き先の番号を置き、呼び出しの後の検査で 1 段ずつ戻って伝える（ir.rs の checked）

fn __aborting() -> bool {
    __load(__static() + 24) != 0
}

// 処理中の handle: [外側の handle][番号][操作の名前の Vec][腕の Vec]
fn __handle_push(ops: int, arms: int) -> int {
    let st = __static()
    let id = __load(st + 56) + 1
    __store(st + 56, id)
    let h = __alloc(48)
    __store(h, __load(st + 48))
    __store(h + 8, id)
    __setv(h + 16, ops)
    __setv(h + 32, arms)
    __store(st + 48, h)
    id
}

// 本体を終えた handle を外す。この handle への中断なら、腕の値が handle の値になる
fn __handle_end(id: int, v: int) -> int {
    let st = __static()
    __store(st + 48, __load(__load(st + 48)))
    if __load(st + 24) == id {
        __store(st + 24, 0)
        return __getv(st + 32)
    }
    v
}

// resume した腕の値は構築子の番号 -1 の値で表す
fn __resume(v: int) -> int {
    let p = __data_new(0 - 1, 1)
    __setv(p + 16, v)
    __mkv(5, p)
}

// 操作を、内側の handle から探して腕を呼ぶ。腕はその handle より外で処理する
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
                let r = arm(args)
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
    __die('操作を処理する handle が無い')
}
