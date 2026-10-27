// 位置付きのエラー
use crate::lexer::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub msg: String,
    // (先頭のバイト位置, 長さ)
    pub span: Option<(usize, usize)>,
    // 直し方の提案
    pub help: Option<String>,
    // 関係する別の位置（効果を起こした操作など）
    pub origin: Option<(usize, usize)>,
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

    // 提案がまだ無ければ付ける
    pub fn help(mut self, h: String) -> Self {
        if self.help.is_none() {
            self.help = Some(h);
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
        Error {
            msg,
            span: None,
            help: None,
            origin: None,
        }
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
            help: e.help,
            origin: e.origin,
        })
    }
}
