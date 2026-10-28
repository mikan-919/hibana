// フォーマッタ。src/fmt.rs と同じ整形をする。トークンの並びと改行は変えず、字下げ・行内の空白・空行・コメントの位置だけを整える
use "./lexer.hb".*
use std.*

fn punct(t: Token) -> str { match t.tok { TPunct(p) => p, _ => "" } }

fn is_open(p: str) -> bool { p == "(" || p == "[" || p == "{" }

fn is_close_p(p: str) -> bool { p == ")" || p == "]" || p == "}" }

// 前後に空白を置く演算子
fn is_bin(p: str) -> bool { has(["=", "==", "!=", "<=", ">=", "&&", "||", "=>", "->"], p) }

// 新しい宣言や文を始めるキーワード
fn is_starts(p: str) -> bool {
    has(["fn", "let", "type", "syntax", "macro", "effect", "use", "module", "extern", "partial", "return", "trait", "impl", "struct", "enum"], p)
}

fn is_ident(t: Tok) -> bool { match t { TIdent(n) => true, _ => false } }

// 値の終わりになるトークン
fn ends_value(t: Token) -> bool {
    match t.tok {
        TInt(n) => true,
        TStr(s) => true,
        TChar(c) => true,
        TIdent(n) => true,
        _ => has([")", "]", "}", "true", "false"], punct(t)),
    }
}

// i 番目のトークン。範囲の外は TEof
fn tk(ts: Vec<Token>, i: int) -> Token {
    if i >= 0 && i < len(ts) { ts[i] } else { Token { tok: TEof, line: 0, col: 0, gap: false, nl: false, s: 0, e: 0 } }
}

// 文字の並びの a から b の手前まで
fn range(cs: Vec<str>, a: int, b: int) -> str {
    let mut s = ""
    for i in a..b { s = s + char_at(cs, i) }
    s
}

fn repeat(s: str, n: int) -> str {
    let mut r = ""
    for i in 0..n { r = r + s }
    r
}

fn trim_end(s: str) -> str {
    let cs = chars(s)
    let mut n = len(cs)
    while n > 0 && is_space(char_at(cs, n - 1)) { n = n - 1 }
    range(cs, 0, n)
}

fn trim(s: str) -> str {
    let cs = chars(s)
    let mut a = 0
    let mut b = len(cs)
    while a < b && is_space(char_at(cs, a)) { a = a + 1 }
    while b > a && is_space(char_at(cs, b - 1)) { b = b - 1 }
    range(cs, a, b)
}

fn cat(xs: Vec<str>, sep: str) -> str {
    let mut r = ""
    let mut first = true
    for x in xs { r = if first { x } else { r + sep + x }; first = false }
    r
}

fn init<A>(xs: Vec<A>) -> Vec<A> {
    let mut r: Vec<A> = []
    for i in 0..len(xs) { if i + 1 < len(xs) { r = push(r, xs[i]) } }
    r
}

fn last_i(xs: Vec<int>) -> int { if len(xs) > 0 { xs[len(xs) - 1] } else { 0 } }

fn last_b(xs: Vec<bool>) -> bool { if len(xs) > 0 { xs[len(xs) - 1] } else { false } }

fn at_i(xs: Vec<int>, i: int) -> int { if i >= 0 && i < len(xs) { xs[i] } else { 0 } }

// k 番目が JSX の開始タグ（< の直後に空白の無い識別子）か
fn jsx_start(ts: Vec<Token>, k: int) -> bool {
    punct(tk(ts, k)) == "<" && is_ident(tk(ts, k + 1).tok) && !tk(ts, k + 1).gap
}

// i 番目から始まる JSX の要素（<tag ... </tag>）の最後のトークンの番号。無ければ -1
// JSX の本文は空白も文字列になるので、要素はソースのまま写す
fn jsx_end(ts: Vec<Token>, i: int) -> int {
    let p = tk(ts, i - 1)
    if !jsx_start(ts, i) || (i > 0 && (ends_value(p) || punct(p) == ">")) {
        -1
    } else {
        let mut depth = 0
        let mut k = i
        let mut r = -1
        let mut going = true
        while going && k < len(ts) {
            if jsx_start(ts, k) {
                depth = depth + 1
            } else if punct(tk(ts, k)) == "</" {
                depth = depth - 1
                if depth == 0 {
                    going = false
                    if k + 2 < len(ts) { r = k + 2 }
                }
            }
            k = k + 1
        }
        r
    }
}

struct Cmt { nl: int, text: str }
struct Cms { items: Vec<Cmt>, nl: int }

// a から b の手前までのコメント (前の改行の数, 綴り) と、最後のコメントの後の改行の数
fn comments(cs: Vec<str>, a: int, b: int) -> Cms {
    let mut out: Vec<Cmt> = []
    let mut nl = 0
    let mut i = a
    while i < b {
        if char_at(cs, i) == "/" && char_at(cs, i + 1) == "/" {
            let mut j = i
            while j < b && char_at(cs, j) != "\n" { j = j + 1 }
            out = push(out, Cmt { nl: nl, text: trim_end(range(cs, i, j)) })
            nl = 0
            i = j
        } else if char_at(cs, i) == "/" && char_at(cs, i + 1) == "*" {
            let mut j = i + 2
            while j < b && !ahead(cs, j, "*/") { j = j + 1 }
            j = if j < b { j + 2 } else { b }
            out = push(out, Cmt { nl: nl, text: range(cs, i, j) })
            nl = 0
            i = j
        } else {
            if char_at(cs, i) == "\n" { nl = nl + 1 }
            i = i + 1
        }
    }
    Cms { items: out, nl: nl }
}

// i 番目の ) が $( を閉じるか
fn rep_close(ts: Vec<Token>, i: int) -> bool {
    let mut depth = 0
    let mut k = i - 1
    let mut r = false
    let mut going = true
    while going && k >= 0 {
        let p = punct(tk(ts, k))
        if is_close_p(p) {
            depth = depth + 1
        } else if is_open(p) && depth > 0 {
            depth = depth - 1
        } else if p == "(" {
            r = k > 0 && punct(tk(ts, k - 1)) == "$"
            going = false
        } else if p == "[" || p == "{" {
            going = false
        }
        k = k - 1
    }
    r
}

// 整形でトークンの並びと改行の位置が変わっていないか
fn same_tokens(a: Vec<Token>, b: Vec<Token>) -> bool {
    let mut ok = len(a) == len(b)
    for i in 0..len(a) { if ok && (tk(a, i).tok != tk(b, i).tok || tk(a, i).nl != tk(b, i).nl) { ok = false } }
    ok
}

partial fn format(src: str) -> Result<str, str> {
    match lex(src) {
        Err(e) => Err(to_str(e.line) + ":" + to_str(e.col) + " " + e.msg),
        Ok(ts) => {
            let cs = chars(src)
            let mut out: Vec<str> = []
            // 開いた括弧ごとの中身の字下げの深さと、$ の直後の括弧か。$( と ${ の中は元の空白のまま
            let mut sd: Vec<int> = [0]
            let mut ss: Vec<bool> = [false]
            // 続きの行（演算子で終わった行の次や | . で始まる行）を始めた括弧の深さ。無ければ -1
            let mut cont = -1
            // syntax の宣言の中か
            let mut in_syntax = false
            let mut line_indent = 0
            // 今の行の初めに開いていた括弧の数
            let mut line_base = 1
            let mut prev = -1
            let mut prev_end = 0
            let mut prev_bin = false
            let mut prev_rep = false
            let mut i = 0
            let mut done = false
            while !done && i < len(ts) {
                let t = tk(ts, i)
                let cm = comments(cs, prev_end, t.s)
                let after_open = prev >= 0 && is_open(punct(tk(ts, prev)))
                let is_close = is_close_p(punct(t))
                let mut fresh_line = prev < 0
                let mut k = 0
                for c in cm.items {
                    if c.nl == 0 && k == 0 && prev >= 0 {
                        out = push(out, " ")
                    } else if len(out) > 0 {
                        let top = last_i(sd)
                        out = push(out, "\n")
                        if c.nl >= 2 && !after_open { out = push(out, "\n") }
                        out = push(out, repeat("    ", top))
                        line_indent = top
                        line_base = len(sd)
                    }
                    out = push(out, c.text)
                    fresh_line = false
                    k = k + 1
                }
                if prev < 0 && punct(t) == "syntax" { in_syntax = true }
                if t.tok == TEof {
                    done = true
                } else {
                    let line_end = len(cm.items) > 0 && starts_with(cm.items[len(cm.items) - 1].text, "//")
                    if cm.nl > 0 || line_end {
                        // 行頭に並んだ閉じ括弧のうち、一番外側の括弧を開いた行の深さにする
                        let mut closers = 0
                        let mut kk = i
                        let mut counting = true
                        while counting && kk < len(ts) {
                            let x = tk(ts, kk)
                            if is_close_p(punct(x)) && (kk == i || !x.gap) { closers = closers + 1; kk = kk + 1 } else { counting = false }
                        }
                        if is_close { sd = init(sd); ss = init(ss) }
                        let mut top = at_i(sd, len(sd) - (if closers > 1 { closers } else { 1 }))
                        if closers == 0 { top = last_i(sd) }
                        let blank = cm.nl >= 2 && !is_close && (!after_open || len(cm.items) > 0)
                        let starts = (prev >= 0 && is_bin(punct(tk(ts, prev)))) || punct(t) == "|" || punct(t) == "."
                        if blank || is_starts(punct(t)) { in_syntax = punct(t) == "syntax" }
                        let keeps = in_syntax && cont == len(sd) && closers == 0 && !is_starts(punct(t))
                        cont = if starts || keeps { len(sd) } else { -1 }
                        if cont >= 0 { top = top + 1 }
                        out = push(out, "\n")
                        if blank { out = push(out, "\n") }
                        out = push(out, repeat("    ", top))
                        line_indent = top
                        line_base = len(sd)
                    } else if !fresh_line {
                        let pp = punct(tk(ts, prev))
                        let c = punct(t)
                        let had = t.s > prev_end
                        let splice = last_b(ss)
                        let space = if prev_rep || (splice && (pp == "(" || pp == "{" || c == ")" || c == "}")) {
                            had
                        } else if c == "," || c == ")" || c == "]" || c == ";" || c == ":" || pp == "(" || pp == "[" {
                            false
                        } else if pp == "}" && c == "else" {
                            true
                        } else if pp == "," {
                            !(c == "*" || c == "+" || c == "?")
                        } else if c == "{" {
                            pp != "$"
                        } else if pp == "{" {
                            c != "}"
                        } else if c == "}" || is_bin(c) || prev_bin {
                            true
                        } else {
                            had
                        }
                        if is_close {
                            sd = init(sd)
                            ss = init(ss)
                            if len(sd) < line_base { line_base = len(sd) }
                        }
                        if space { out = push(out, " ") }
                    }
                    let end = jsx_end(ts, i)
                    if end >= 0 {
                        // JSX の要素はソースのまま
                        out = push(out, range(cs, t.s, tk(ts, end).e))
                        prev = end
                        prev_end = tk(ts, end).e
                        prev_bin = false
                        prev_rep = false
                        i = end + 1
                    } else {
                        out = push(out, range(cs, t.s, t.e))
                        let c = punct(t)
                        prev_rep = false
                        if is_open(c) {
                            let level = if len(sd) > line_base { last_i(sd) } else { line_indent + 1 }
                            sd = push(sd, level)
                            ss = push(ss, c != "[" && prev >= 0 && punct(tk(ts, prev)) == "$")
                        } else if is_close {
                            prev_rep = c == ")" && rep_close(ts, i)
                        }
                        prev_bin = is_bin(c)
                        prev = i
                        prev_end = t.e
                        i = i + 1
                    }
                }
            }
            let mut lines: Vec<str> = []
            for l in split(cat(out, ""), "\n") { lines = push(lines, trim_end(l)) }
            let res = trim(cat(lines, "\n")) + "\n"
            match lex(res) {
                Err(e) => Err(e.msg),
                Ok(after) => if same_tokens(ts, after) { Ok(res) } else { Err("整形でトークンの並びが変わった（フォーマッタの誤り）") },
            }
        },
    }
}

test "layout" {
    let src = "fn add(a:int,b :int)->int{   a+b }  // 足す\n\n\n\nfn main(){\nlet x=add( 1,2 )\n      if x==3 {print( \"ok\" )}else{\n  print(\"ng\")\n}\n    /* c */\n  let ok = x == 3 ||\n  x == 4\n  print(<p>a   b {x}</p>)\n  print(web.json(f([\n    1\n    ])))\n\n}\n"
    let want = "fn add(a:int, b:int) -> int { a+b } // 足す\n\nfn main() {\n    let x = add(1, 2)\n    if x == 3 { print(\"ok\") } else {\n        print(\"ng\")\n    }\n    /* c */\n    let ok = x == 3 ||\n        x == 4\n    print(<p>a   b {x}</p>)\n    print(web.json(f([\n        1\n    ])))\n}\n"
    assert_eq(match format(src) { Ok(s) => s, Err(e) => e }, want)
}
