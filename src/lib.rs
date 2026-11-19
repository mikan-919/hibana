//! hibana のコンパイラーとビルドの各段階。
pub mod backend;
pub mod check;
pub mod cli;
pub mod error;
pub mod frontend;
pub mod pipeline;
pub mod tooling;

// 既存の利用側は、機能別ディレクトリへの移動後も同じパスで参照できる。
pub use backend::{eval, ir, js, native, wasm};
pub use check::{smt, typeck};
pub use frontend::{lexer, meta, parser};
pub use tooling::fmt;

pub(crate) use frontend::{resolve, ts};

use std::io::Write;

fn checked(src: &str, kind: pipeline::Kind) -> error::R<pipeline::Checked> {
    let modules = pipeline::load(src, &parser::base_dir())?;
    pipeline::check(pipeline::parse(modules)?, kind)
}

pub fn run(src: &str, out: &mut dyn Write) -> error::R<eval::Value> {
    let checked = checked(src, pipeline::Kind::Executable)?;
    eval::run_fn(&checked.program, "main", out)
}

pub fn to_js(src: &str) -> error::R<String> {
    pipeline::emit_js(&checked(
        src,
        pipeline::Kind::Executable,
    )?)
}

pub fn to_ir(src: &str) -> error::R<String> {
    Ok(pipeline::lower(&checked(src, pipeline::Kind::Executable)?, None)?.to_string())
}

pub fn to_wasm(src: &str) -> error::R<Vec<u8>> {
    let main = ["main".to_string()];
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Executable)?, Some(&main))?;
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

/// WASM の JavaScript 接続コードに対応する TypeScript の型宣言を作る。
pub fn to_ts_declarations(src: &str, exports: &[String]) -> error::R<String> {
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Library)?, Some(exports))?;
    pipeline::emit_ts_declarations(&ir, exports)
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
    let javascript = pipeline::emit_js_glue(&ir, exports)?;
    Ok(WebOutput {
        wasm: wasm::emit_with_glue(&ir, exports),
        javascript,
    })
}

/// libc を使わない静的な ELF の実行ファイルを作る。main から辿れる関数だけを含める。
pub fn to_native(src: &str) -> error::R<Vec<u8>> {
    let main = ["main".to_string()];
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Executable)?, Some(&main))?;
    native::emit(&ir)
}
