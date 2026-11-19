#![allow(dead_code)]

use hibana::{error::R, eval::Value};

pub struct Artifacts {
    pub wasm: Vec<u8>,
    pub javascript: String,
}

pub fn build(source: &str, exports: &[String]) -> R<Artifacts> {
    let (wasm, abi) = hibana::compile_wasm_library(source, exports)?;
    Ok(Artifacts { wasm, javascript: javascript(&abi)? })
}

pub fn javascript(abi: &str) -> R<String> {
    render("javascript", abi)
}

pub fn typescript(abi: &str) -> R<String> {
    render("typescript", abi)
}

fn render(function: &str, abi: &str) -> R<String> {
    let abi = serde_json::to_string(abi).unwrap();
    let script = format!("use std::wasm_web\nfn main() -> String {{ wasm_web.{function}({abi}) }}");
    let Value::Str(output) = hibana::run(&script, &mut Vec::new())? else {
        unreachable!("生成関数は文字列を返す");
    };
    Ok(output.to_string())
}
