use crate::error::{Error, R};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    Ident(String),
    // 記号とキーワードを共に表す
    Punct(&'static str),
    Eof,
}

impl Tok {
    // パターンのリテラルと照合する時の綴り
    pub fn text(&self) -> Option<&str> {
        match self {
            Tok::Ident(s) => Some(s),
            Tok::Punct(p) => Some(p),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    // 直前のトークンとの間に空白・改行・コメントがあるか（"~" の判定に使う）
    pub gap_before: bool,
    // 直前のトークンとの間に改行があるか（文の区切りに使う）
    pub nl_before: bool,
}

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "else", "while", "return", "true", "false", "syntax", "macro",
    "type", "match", "quote", "partial", "for", "in", "use",
];
// 最長一致のため、2文字の記号を先に置く
const PUNCTS: &[&str] = &[
    "->", "=>", "..", "==", "!=", "<=", ">=", "&&", "||", "</", "(", ")", "{", "}", "[", "]", ",",
    ";", ":", ".", "=", "<", ">", "+", "-", "*", "/", "%", "!", "@", "$", "~", "|", "?",
];

struct Lx<'a> {
    src: &'a str,
    i: usize,
    line: usize,
    col: usize,
}

impl Lx<'_> {
    fn pos(&self) -> Pos {
        Pos {
            line: self.line,
            column: self.col,
            offset: self.i,
        }
    }
    fn rest(&self) -> &str {
        &self.src[self.i..]
    }
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn peek2(&self) -> Option<char> {
        self.rest().chars().nth(1)
    }
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }
    fn err<T>(&self, msg: &str) -> R<T> {
        let p = self.pos();
        Err(Error::from(msg).at(Span {
            start: p,
            end: Pos {
                offset: p.offset + 1,
                ..p
            },
        }))
    }

    // 空白とコメントを読み飛ばし、読み飛ばしたかを返す
    fn skip(&mut self) -> R<bool> {
        let mut gap = false;
        loop {
            if self.peek().is_some_and(|c| c.is_whitespace()) {
                self.bump();
            } else if self.rest().starts_with("//") {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.bump();
                }
            } else if self.rest().starts_with("/*") {
                let start = self.pos();
                self.i += 2;
                while !self.rest().starts_with("*/") {
                    if self.bump().is_none() {
                        return Err(Error::from("ブロックコメントが閉じていない").at(Span {
                            start,
                            end: Pos {
                                offset: start.offset + 2,
                                ..start
                            },
                        }));
                    }
                }
                self.i += 2;
            } else {
                return Ok(gap);
            }
            gap = true;
        }
    }

    fn number(&mut self) -> R<Tok> {
        let s = self.i;
        if self.rest().starts_with("0x") || self.rest().starts_with("0X") {
            self.i += 2;
            while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                self.bump();
            }
            return match i64::from_str_radix(&self.src[s + 2..self.i], 16) {
                Ok(n) => Ok(Tok::Int(n)),
                Err(_) => self.err("16進数が不正"),
            };
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
        }
        if self.peek() == Some('.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
            return Ok(Tok::Float(self.src[s..self.i].parse().unwrap()));
        }
        match self.src[s..self.i].parse() {
            Ok(n) => Ok(Tok::Int(n)),
            Err(_) => self.err("整数が大きすぎる"),
        }
    }

    // 文字列・文字リテラルの中の1文字（エスケープを解決する）
    fn quoted_char(&mut self) -> R<char> {
        match self.bump() {
            None => self.err("リテラルが閉じていない"),
            Some('\\') => match self.bump() {
                Some('n') => Ok('\n'),
                Some('t') => Ok('\t'),
                Some('r') => Ok('\r'),
                Some('0') => Ok('\0'),
                Some(c @ ('\\' | '"' | '\'')) => Ok(c),
                _ => self.err("不明なエスケープ"),
            },
            Some(c) => Ok(c),
        }
    }
}

pub fn lex(src: &str) -> R<Vec<Token>> {
    let mut lx = Lx {
        src,
        i: 0,
        line: 1,
        col: 1,
    };
    let mut out: Vec<Token> = Vec::new();
    loop {
        // ファイル先頭のトークンは gap_before = false
        let line0 = lx.line;
        let gap = lx.skip()? && !out.is_empty();
        let nl = lx.line > line0;
        let start = lx.pos();
        let Some(c) = lx.peek() else {
            out.push(Token {
                tok: Tok::Eof,
                span: Span { start, end: start },
                gap_before: gap,
                nl_before: nl,
            });
            return Ok(out);
        };
        let tok = if c.is_ascii_digit() {
            lx.number()?
        } else if c.is_ascii_alphabetic() || c == '_' {
            while lx
                .peek()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                lx.bump();
            }
            let w = &src[start.offset..lx.i];
            match KEYWORDS.iter().find(|k| **k == w) {
                Some(k) => Tok::Punct(k),
                None => Tok::Ident(w.to_string()),
            }
        } else if c == '"' {
            lx.bump();
            let mut s = String::new();
            while lx.peek() != Some('"') {
                s.push(lx.quoted_char()?);
            }
            lx.bump();
            Tok::Str(s)
        } else if c == '\'' {
            lx.bump();
            let ch = lx.quoted_char()?;
            if lx.bump() != Some('\'') {
                return lx.err("文字リテラルが閉じていない");
            }
            Tok::Char(ch)
        } else if let Some(p) = PUNCTS.iter().find(|p| lx.rest().starts_with(**p)) {
            lx.i += p.len();
            lx.col += p.len();
            Tok::Punct(p)
        } else {
            return lx.err(&format!("不明な文字 {c:?}"));
        };
        out.push(Token {
            tok,
            span: Span {
                start,
                end: lx.pos(),
            },
            gap_before: gap,
            nl_before: nl,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gap_and_literals() {
        let ts = lex("a/* c */b ~c</x 0xFF 2.5 \"h\\n\" 'x'").unwrap();
        let gaps: Vec<bool> = ts.iter().map(|t| t.gap_before).collect();
        assert_eq!(
            gaps,
            [
                false, true, true, false, false, false, true, true, true, true, false
            ]
        );
        assert_eq!(ts[4].tok, Tok::Punct("</"));
        assert_eq!(ts[6].tok, Tok::Int(255));
        assert_eq!(ts[7].tok, Tok::Float(2.5));
        assert_eq!(ts[8].tok, Tok::Str("h\n".into()));
        assert_eq!(ts[9].tok, Tok::Char('x'));
    }
}
