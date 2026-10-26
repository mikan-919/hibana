#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Ident(String),
    // 記号とキーワードを共に表す
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
}

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "else", "while", "return", "true", "false",
];
// 長い記号を先に置く（"->" を "-" より先に照合するため）
const PUNCTS: &[&str] = &[
    "->", "==", "!=", "<=", ">=", "&&", "||", "(", ")", "{", "}", ",", ";", ":", "=", "<", ">",
    "+", "-", "*", "/", "%", "!",
];

pub fn lex(src: &str) -> Result<Vec<Token>, String> {
    let b = src.as_bytes();
    let (mut out, mut line, mut i) = (Vec::new(), 1, 0);
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c.is_ascii_digit() {
            let s = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            let n = src[s..i]
                .parse()
                .map_err(|_| format!("{line}行: 整数が大きすぎる"))?;
            out.push(Token {
                tok: Tok::Int(n),
                line,
            });
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let w = &src[s..i];
            let tok = match KEYWORDS.iter().find(|k| **k == w) {
                Some(k) => Tok::Punct(k),
                None => Tok::Ident(w.to_string()),
            };
            out.push(Token { tok, line });
        } else if let Some(p) = PUNCTS.iter().find(|p| src[i..].starts_with(**p)) {
            i += p.len();
            out.push(Token {
                tok: Tok::Punct(p),
                line,
            });
        } else {
            let ch = src[i..].chars().next().unwrap();
            return Err(format!("{line}行: 不明な文字 {ch:?}"));
        }
    }
    out.push(Token {
        tok: Tok::Eof,
        line,
    });
    Ok(out)
}
