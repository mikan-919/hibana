//! hibana のコンパイラーとビルドの各段階。
pub mod build;
pub mod c;
pub mod cli;
pub mod error;
pub mod eval;
pub mod fmt;
pub mod index;
pub mod ir;
pub mod js;
pub mod lexer;
pub mod lsp;
pub mod meta;
pub mod parser;
pub mod pipeline;
pub mod pkg;
mod resolve;
mod scripts;
pub mod smt;
mod tools;
mod ts;
pub mod typeck;
pub mod wasm;

use std::io::Write;

fn checked(src: &str, kind: pipeline::Kind) -> error::R<pipeline::Checked> {
    let expanded = parser::with_uses(src)?;
    pipeline::check(pipeline::parse(&expanded)?, kind)
}

pub fn run(src: &str, out: &mut dyn Write) -> error::R<eval::Value> {
    let checked = checked(src, pipeline::Kind::Executable)?;
    eval::run(&checked.program, out)
}

pub fn to_js(src: &str) -> error::R<String> {
    Ok(pipeline::emit_js(&checked(
        src,
        pipeline::Kind::Executable,
    )?))
}

pub fn to_ir(src: &str) -> error::R<String> {
    Ok(pipeline::lower(&checked(src, pipeline::Kind::Executable)?, None)?.to_string())
}

pub fn to_wasm(src: &str) -> error::R<Vec<u8>> {
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Executable)?, None)?;
    pipeline::emit_wasm(&ir, None)
}

pub fn to_wasm_library(src: &str, exports: &[String]) -> error::R<Vec<u8>> {
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Library)?, Some(exports))?;
    pipeline::emit_wasm(&ir, Some(exports))
}

pub fn to_js_glue(src: &str, exports: &[String]) -> error::R<String> {
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Library)?, Some(exports))?;
    pipeline::emit_js_glue(&ir, exports)
}

/// Web 向けの WASM と、それを呼び出す ES モジュール。
pub struct WebOutput {
    pub wasm: Vec<u8>,
    pub javascript: String,
}

/// ソースを一度だけ型検査し、指定した関数を公開する Web 用成果物を作る。
pub fn to_web(src: &str, exports: &[String]) -> error::R<WebOutput> {
    if exports.is_empty() {
        return Err("Web の公開関数を指定してください".into());
    }
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Library)?, Some(exports))?;
    Ok(WebOutput {
        wasm: pipeline::emit_wasm(&ir, Some(exports))?,
        javascript: pipeline::emit_js_glue(&ir, exports)?,
    })
}

pub fn to_c(src: &str) -> error::R<String> {
    pipeline::emit_c(&checked(src, pipeline::Kind::Executable)?)
}
