// 位置付きのエラー
use crate::lexer::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub msg: String,
    // (先頭のバイト位置, 長さ)
    pub span: Option<(usize, usize)>,
}

pub type R<T> = Result<T, Error>;

impl Error {
    // 位置がまだ無ければ付ける。内側の式の位置を優先するため
    pub fn at(mut self, s: Span) -> Self {
        if self.span.is_none() {
            self.span = Some((s.start.offset, s.end.offset - s.start.offset));
        }
        self
    }

    #[cfg(test)]
    pub fn contains(&self, s: &str) -> bool {
        self.msg.contains(s)
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Error { msg, span: None }
    }
}

impl From<&str> for Error {
    fn from(msg: &str) -> Self {
        msg.to_string().into()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.msg)
    }
}

// 位置を保ったままメッセージに前置きを付ける
pub trait Ctx<T> {
    fn ctx(self, f: impl FnOnce(&str) -> String) -> R<T>;
}

impl<T> Ctx<T> for R<T> {
    fn ctx(self, f: impl FnOnce(&str) -> String) -> R<T> {
        self.map_err(|e| Error {
            msg: f(&e.msg),
            span: e.span,
        })
    }
}
