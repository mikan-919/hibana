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

/// 同じ型検査結果から WASM ライブラリと受け渡し規約の JSON を作る。
pub fn compile_wasm_library(src: &str, exports: &[String]) -> error::R<(Vec<u8>, String)> {
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Library)?, Some(exports))?;
    let abi = pipeline::wasm_abi(&ir, exports)?;
    let wasm = pipeline::emit_wasm(&ir, Some(exports))?;
    Ok((wasm, abi))
}

/// libc を使わない静的な ELF の実行ファイルを作る。main から辿れる関数だけを含める。
pub fn to_native(src: &str) -> error::R<Vec<u8>> {
    let main = ["main".to_string()];
    let ir = pipeline::lower(&checked(src, pipeline::Kind::Executable)?, Some(&main))?;
    native::emit(&ir)
}
