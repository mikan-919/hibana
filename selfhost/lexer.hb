// 字句解析器。src/lexer.rs と同じトークンの並びを作る
use std.*

derive(Eq, Show)
enum Tok { TInt(n: int), TStr(s: str), TChar(c: str), TIdent(name: str), TPunct(p: str), TEof }

derive(Eq, Show)
struct Token { tok: Tok, line: int, col: int, gap: bool, nl: bool }

struct LexError { msg: str, line: int, col: int }

fn keywords() -> Vec<str> {
    ["fn", "let", "mut", "if", "else", "while", "return", "true", "false", "syntax", "macro",
     "type", "match", "quote", "partial", "for", "in", "use", "module", "extern", "effect",
     "handle", "trait", "impl", "struct", "enum"]
}

// 最長一致のため、2文字の記号を先に置く
fn puncts() -> Vec<str> {
    ["->", "=>", "..", "==", "!=", "<=", ">=", "&&", "||", "</", "(", ")", "{", "}", "[", "]", ",",
     ";", ":", ".", "=", "<", ">", "+", "-", "*", "/", "%", "!", "@", "$", "~", "|", "?"]
}

// i 番目の文字。範囲の外は ""
fn char_at(cs: Vec<str>, i: int) -> str {
    if i >= 0 && i < len(cs) { cs[i] } else { "" }
}

fn is_digit(c: str) -> bool { code(c) >= 48 && code(c) <= 57 }

fn is_hex(c: str) -> bool {
    is_digit(c) || (code(c) >= 97 && code(c) <= 102) || (code(c) >= 65 && code(c) <= 70)
}

fn is_alpha(c: str) -> bool {
    (code(c) >= 97 && code(c) <= 122) || (code(c) >= 65 && code(c) <= 90) || c == "_"
}

fn is_space(c: str) -> bool { c == " " || c == "\n" || c == "\t" || c == "\r" }

fn digit(c: str) -> int {
    if is_digit(c) { code(c) - 48 } else if code(c) >= 97 { code(c) - 87 } else { code(c) - 55 }
}

// cs の i から s が続くか
fn ahead(cs: Vec<str>, i: int, s: str) -> bool {
    let ps = chars(s)
    let mut k = 0
    let mut ok = true
    while ok && k < len(ps) {
        ok = char_at(cs, i + k) == char_at(ps, k)
        k = k + 1
    }
    ok
}

fn lex(src: str) -> Result<Vec<Token>, LexError> {
    let cs = chars(src)
    let kws = keywords()
    let ps = puncts()
    let mut out: Vec<Token> = []
    let mut i = 0
    let mut line = 1
    // 行頭の位置。列は i - bol + 1
    let mut bol = 0
    let mut err = ""
    let mut done = false
    while !done && err == "" {
        // 空白とコメントを読み飛ばす。ファイル先頭のトークンは gap = false
        let line0 = line
        let mut gap = false
        let mut skipping = true
        while skipping {
            let c = char_at(cs, i)
            if is_space(c) {
                if c == "\n" { line = line + 1; bol = i + 1 }
                i = i + 1
                gap = true
            } else if c == "/" && char_at(cs, i + 1) == "/" {
                while char_at(cs, i) != "\n" && char_at(cs, i) != "" { i = i + 1 }
                gap = true
            } else if c == "/" && char_at(cs, i + 1) == "*" {
                i = i + 2
                while !ahead(cs, i, "*/") && char_at(cs, i) != "" {
                    if char_at(cs, i) == "\n" { line = line + 1; bol = i + 1 }
                    i = i + 1
                }
                if char_at(cs, i) == "" { err = "ブロックコメントが閉じていない" }
                i = i + 2
                gap = true
            } else {
                skipping = false
            }
        }
        let gap = gap && len(out) > 0
        let nl = line > line0
        let l0 = line
        let c0 = i - bol + 1
        let c = char_at(cs, i)
        let mut tok = TEof
        if err != "" {
        } else if c == "" {
            done = true
        } else if c == "0" && (char_at(cs, i + 1) == "x" || char_at(cs, i + 1) == "X") {
            i = i + 2
            let mut n = 0
            while is_hex(char_at(cs, i)) { n = n * 16 + digit(char_at(cs, i)); i = i + 1 }
            tok = TInt(n)
        } else if is_digit(c) {
            let mut n = 0
            while is_digit(char_at(cs, i)) { n = n * 10 + digit(char_at(cs, i)); i = i + 1 }
            tok = TInt(n)
        } else if is_alpha(c) {
            let mut w = ""
            while is_alpha(char_at(cs, i)) || is_digit(char_at(cs, i)) { w = w + char_at(cs, i); i = i + 1 }
            tok = if has(kws, w) { TPunct(w) } else { TIdent(w) }
        } else if c == "\"" || c == "'" {
            i = i + 1
            let mut s = ""
            while char_at(cs, i) != c && err == "" {
                let d = char_at(cs, i)
                if d == "" {
                    err = "リテラルが閉じていない"
                } else if d == "\\" {
                    let e = char_at(cs, i + 1)
                    if e == "n" { s = s + "\n" } else if e == "t" { s = s + "\t" } else if e == "r" { s = s + "\r" }
                    else if e == "\\" || e == "\"" || e == "'" { s = s + e } else { err = "不明なエスケープ" }
                    i = i + 2
                } else {
                    if d == "\n" { line = line + 1; bol = i + 1 }
                    s = s + d
                    i = i + 1
                }
            }
            i = i + 1
            tok = if c == "\"" { TStr(s) } else { TChar(s) }
        } else {
            // 2 文字の記号を先に試す
            let two = c + char_at(cs, i + 1)
            if has(ps, two) { tok = TPunct(two); i = i + 2 } else if has(ps, c) { tok = TPunct(c); i = i + 1 } else { err = "不明な文字 " + c }
        }
        if err == "" {
            out = push(out, Token { tok: tok, line: l0, col: c0, gap: gap, nl: nl })
        }
    }
    if err == "" { Ok(out) } else { Err(LexError { msg: err, line: line, col: i - bol + 1 }) }
}

fn has(xs: Vec<str>, s: str) -> bool {
    let mut r = false
    for x in xs { if x == s { r = true } }
    r
}

fn kinds(src: str) -> Vec<Tok> {
    match lex(src) { Ok(ts) => map(ts, fn(t: Token) -> Tok { t.tok }), Err(e) => panic(e.msg) }
}

test "literals" {
    assert_eq(kinds("a 0xFF 25 \"h\\n\" 'x' fn"), [TIdent("a"), TInt(255), TInt(25), TStr("h\n"), TChar("x"), TPunct("fn"), TEof])
}

test "puncts and gaps" {
    let ts = match lex("a/* c */b ~c</x") { Ok(ts) => ts, Err(e) => panic(e.msg) }
    assert_eq(map(ts, fn(t: Token) -> bool { t.gap }), [false, true, true, false, false, false, false])
    assert_eq(map(ts, fn(t: Token) -> Tok { t.tok }), [TIdent("a"), TIdent("b"), TPunct("~"), TIdent("c"), TPunct("</"), TIdent("x"), TEof])
}

test "positions" {
    let ts = match lex("a\n  bc // x\nd") { Ok(ts) => ts, Err(e) => panic(e.msg) }
    assert_eq(map(ts, fn(t: Token) -> int { t.line * 100 + t.col }), [101, 203, 301, 302])
    assert_eq(map(ts, fn(t: Token) -> bool { t.nl }), [false, true, true, false])
}
