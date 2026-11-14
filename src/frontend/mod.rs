//! 字句・構文解析、マクロ展開、名前解決。
pub mod lexer;
pub mod meta;
pub mod parser;
pub(crate) mod resolve;
pub(crate) mod ts;
