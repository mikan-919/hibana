// 位置付きのエラー
use crate::lexer::Span;
use crate::frontend::source::{SourceFile, SourceId};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub msg: String,
    pub source_id: SourceId,
    pub source: Option<SourceFile>,
    // (先頭のバイト位置, 長さ)
    pub span: Option<(usize, usize)>,
    // 直し方の提案
    pub help: Option<String>,
    // 関係する別の位置（効果を起こした操作など）
    pub origin: Option<(usize, usize)>,
    pub origin_source: Option<SourceId>,
    pub origin_file: Option<SourceFile>,
    pub origin_label: Option<&'static str>,
    // span の行番号。with_line で msg の前に「行 N: 」を付けると None に戻る
    pub line: Option<usize>,
}

pub type R<T> = Result<T, Error>;

impl Error {
    // 位置がまだ無ければ付ける。内側の式の位置を優先するため
    pub fn at(mut self, s: Span) -> Self {
        if self.span.is_none() {
            self.source_id = s.source;
            self.span = Some((s.start.offset, s.end.offset - s.start.offset));
            // 行は 1 から数える。0 は位置の無い Span::default() なので付けない
            self.line = (s.start.line > 0).then_some(s.start.line);
        }
        self
    }

    pub(crate) fn with_sources(mut self, sources: impl IntoIterator<Item = impl AsRef<SourceFile>>) -> Self {
        for source in sources {
            let source = source.as_ref();
            if source.id == self.source_id { self.source = Some(source.clone()); }
            if Some(source.id) == self.origin_source { self.origin_file = Some(source.clone()); }
        }
        self
    }

    // 行番号があれば msg の前に「行 N: 」を付ける。型検査の結果を外へ返す時に一度だけ使う
    pub fn with_line(mut self) -> Self {
        if let Some(n) = self.line.take() {
            self.msg = format!("行 {n}: {}", self.msg);
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
            source_id: SourceId::default(),
            source: None,
            span: None,
            help: None,
            origin: None,
            origin_source: None,
            origin_file: None,
            origin_label: None,
            line: None,
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
            ..e
        })
    }
}
