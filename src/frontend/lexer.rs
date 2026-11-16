use crate::error::{Error, R};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    // `...{式}...`。ソースのままの中身と、文字列と式の並び
    Template(String, Vec<TPart>),
    Ident(String),
    Raw(char),
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
    pub source: super::source::SourceId,
    pub start: Pos,
    pub end: Pos,
}

#[derive(Debug, Clone)]
pub enum TPart {
    Lit(String),
    // { } の中の式のトークン。最後は Eof
    Code(Vec<Token>),
}

// 位置を除いて比べる（整形の前後で同じトークンの並びかを確かめるため）
impl PartialEq for TPart {
    fn eq(&self, o: &Self) -> bool {
        match (self, o) {
            (TPart::Lit(a), TPart::Lit(b)) => a == b,
            (TPart::Code(a), TPart::Code(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.tok == y.tok),
            _ => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    // 直前のトークンとの間に空白・改行・コメントがあるか（"~" の判定に使う）
    pub gap_before: bool,
    // 直前のトークンとの間に改行があるか（文の区切りに使う）
    pub nl_before: bool,
    // マクロの展開で作ったトークンの呼び出し位置。展開したコードのノードはこの位置を指す
    pub site: Option<Span>,
}

pub const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "else", "while", "return", "true", "false", "syntax", "macro",
    "type", "match", "quote", "for", "in", "use", "module", "extern", "effect",
    "handle", "trait", "impl", "struct", "enum",
];
// 最長一致のため、2文字の記号を先に置く
const PUNCTS: &[&str] = &[
    "::", "->", "=>", "..", "**", "==", "!=", "<=", ">=", "&&", "||", "</", "/>", "(", ")", "{", "}", "[", "]", ",",
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
        Err(Error::from(msg).at(Span { source: Default::default(),
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
                self.bump();
                self.bump();
                while !self.rest().starts_with("*/") {
                    if self.bump().is_none() {
                        return Err(Error::from("ブロックコメントが閉じていない").at(Span { source: Default::default(),
                            start,
                            end: Pos {
                                offset: start.offset + 2,
                                ..start
                            },
                        }));
                    }
                }
                self.bump();
                self.bump();
            } else {
                return Ok(gap);
            }
            gap = true;
        }
    }

    fn number(&mut self) -> R<Tok> {
        let s = self.i;
        if self.rest().starts_with("0x") || self.rest().starts_with("0X") {
            self.bump();
            self.bump();
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
                Some(c @ ('\\' | '\'' | '"' | '`' | '{' | '}')) => Ok(c),
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
    lex_from(&mut lx, false)
}

// nested なら `...{式}...` の { の後から読み、対応する } で止まる
fn lex_from(lx: &mut Lx, nested: bool) -> R<Vec<Token>> {
    let src = lx.src;
    let mut out: Vec<Token> = Vec::new();
    let mut depth = 0;
    loop {
        // ファイル先頭のトークンは gap_before = false
        let line0 = lx.line;
        let gap = lx.skip()? && !out.is_empty();
        let nl = lx.line > line0;
        let start = lx.pos();
        if nested && depth == 0 && lx.peek() == Some('}') {
            lx.bump();
            out.push(Token {
                tok: Tok::Eof,
                span: Span { source: Default::default(), start, end: start },
                gap_before: gap,
                nl_before: nl,
                site: None,
            });
            return Ok(paths(out));
        }
        let Some(c) = lx.peek() else {
            if nested {
                return lx.err("文字列の中の { が閉じていない");
            }
            out.push(Token {
                tok: Tok::Eof,
                span: Span { source: Default::default(), start, end: start },
                gap_before: gap,
                nl_before: nl,
                site: None,
            });
            return Ok(paths(out));
        };
        let tok = if c.is_ascii_digit() {
            lx.number()?
        } else if c.is_alphabetic() || c == '_' {
            while lx
                .peek()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                lx.bump();
            }
            let w = &src[start.offset..lx.i];
            match KEYWORDS.iter().find(|k| **k == w) {
                Some(k) => Tok::Punct(k),
                None => Tok::Ident(w.to_string()),
            }
        } else if c == '\'' || c == '"' {
            lx.bump();
            let mut s = String::new();
            while lx.peek() != Some(c) {
                s.push(lx.quoted_char()?);
            }
            lx.bump();
            Tok::Str(s)
        } else if c == '`' {
            lx.bump();
            let mut parts = Vec::new();
            let mut s = String::new();
            loop {
                match lx.peek() {
                    Some('`') => break,
                    Some('{') => {
                        lx.bump();
                        if !s.is_empty() {
                            parts.push(TPart::Lit(std::mem::take(&mut s)));
                        }
                        parts.push(TPart::Code(lex_from(lx, true)?));
                    }
                    _ => s.push(lx.quoted_char()?),
                }
            }
            lx.bump();
            if !s.is_empty() {
                parts.push(TPart::Lit(s));
            }
            Tok::Template(src[start.offset + 1..lx.i - 1].to_string(), parts)
        } else if let Some(p) = PUNCTS.iter().find(|p| lx.rest().starts_with(**p)) {
            lx.i += p.len();
            lx.col += p.len();
            if nested {
                match *p {
                    "{" => depth += 1,
                    "}" => depth -= 1,
                    _ => {}
                }
            }
            Tok::Punct(p)
        } else {
            lx.bump();
            Tok::Raw(c)
        };
        out.push(Token {
            tok,
            span: Span { source: Default::default(),
                start,
                end: lx.pos(),
            },
            gap_before: gap,
            nl_before: nl,
            site: None,
        });
    }
}

// 型の名前::名前 は1つの名前 型の名前__名前 にする（Response::text、fn Param::get の宣言、x.get() の呼び先）
// ponytail: 名前を __ でつなぐだけなので、エラーには Response__text と出る。名前の表を持つ時に専用の名前にする
fn paths(toks: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::with_capacity(toks.len());
    let mut it = toks.into_iter().peekable();
    while let Some(t) = it.next() {
        if let (Some(Token { tok: Tok::Ident(a), .. }), Tok::Punct("::")) = (out.last(), &t.tok) {
            if let Some(Token { tok: Tok::Ident(b), span, .. }) = it.peek() {
                let (name, end) = (format!("{a}__{b}"), span.end);
                let last = out.last_mut().unwrap();
                last.tok = Tok::Ident(name);
                last.span.end = end;
                it.next();
                continue;
            }
        }
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_comment_updates_positions_and_newline_flag() {
        let ts = lex("a/*\nxy*/0xA z").unwrap();
        assert_eq!(ts[1].tok, Tok::Int(10));
        assert_eq!(ts[1].span.start, Pos { line: 2, column: 5, offset: 8 });
        assert_eq!(ts[1].span.end, Pos { line: 2, column: 8, offset: 11 });
        assert!(ts[1].gap_before);
        assert!(ts[1].nl_before);
        assert_eq!(ts[2].span.start, Pos { line: 2, column: 9, offset: 12 });
        assert_eq!(ts[2].span.end, Pos { line: 2, column: 10, offset: 13 });
        assert!(!ts[2].nl_before);
    }

    #[test]
    fn hex_prefix_updates_column_and_offset() {
        let ts = lex("0xA z").unwrap();
        assert_eq!(ts[0].span.start, Pos { line: 1, column: 1, offset: 0 });
        assert_eq!(ts[0].span.end, Pos { line: 1, column: 4, offset: 3 });
        assert_eq!(ts[1].span.start, Pos { line: 1, column: 5, offset: 4 });
        assert_eq!(ts[1].span.end, Pos { line: 1, column: 6, offset: 5 });
    }

    #[test]
    fn gap_and_literals() {
        let ts = lex("a/* c */b ~c</x 0xFF 2.5 'h\\n' `a{x}`").unwrap();
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
        let Tok::Template(raw, parts) = &ts[9].tok else { panic!() };
        assert_eq!(raw, "a{x}");
        assert!(matches!(&parts[..], [TPart::Lit(a), TPart::Code(c)] if a == "a" && c[0].tok == Tok::Ident("x".into())));
        assert_eq!(lex("\"a\"").unwrap()[0].tok, Tok::Str("a".into()));
    }
}
