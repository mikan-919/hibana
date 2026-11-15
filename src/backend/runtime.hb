// IR のランタイム。WASM とネイティブの両方で同じものを使う。
// 型検査は通さず、構文解析だけして IR に変換する。演算子はすべて値の中身（64 ビット整数）の演算になる。
// __load などの __ で始まる組み込みは、IR の命令に直接変換する（src/backend/ir.rs の prim）。
//
// 値の型の番号: 0 unit, 1 int, 2 bool, 3 String, 4 Vec, 5 type の値・構造体, 6 関数, 7 型, 8 Ref
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
    __cat(a, b)
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
