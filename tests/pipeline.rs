#[path = "common/wasm_web.rs"]
mod wasm_web;

use hibana::pipeline::{self, Kind};
use std::path::Path;
use std::process::Command;

#[test]
fn builds_and_links_as_independent_steps() {
    let src = "fn add(a: int, b: int) -> int { a + b }\nfn twice(n: int) -> int { add(n, n) }\nfn log(n: int) { put(n) }\nfn unused(n: int) -> int { n + 999 }";
    let expanded = pipeline::load(src, Path::new(".")).unwrap();
    let parsed = pipeline::parse(expanded).unwrap();
    let checked = pipeline::check(parsed, Kind::Library).unwrap();
    assert_eq!(checked.program().fns.len(), 4);
    let all = pipeline::lower(&checked, None).unwrap();
    // ランタイムの関数（__ で始まる）を除いた、利用者の関数
    let user = |p: &hibana::ir::Program| {
        p.functions()
            .iter()
            .filter(|f| !f.name().starts_with("__"))
            .map(|f| f.name().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(user(&all).len(), 4);
    let exports = vec!["twice".to_string(), "log".to_string()];
    let linked = pipeline::link(&all, &exports).unwrap();
    assert_eq!(user(&linked), ["add", "twice", "log"]);
    let twice = linked
        .functions()
        .iter()
        .find(|f| f.name() == "twice")
        .unwrap();
    assert_eq!(twice.param_types().len(), 1);
    assert!(!twice.body().is_empty());
    assert!(pipeline::link(&all, &["missing".into()]).is_err());
    assert!(pipeline::emit_wasm(&linked, Some(&["unused".into()])).is_err());
    assert!(pipeline::wasm_abi(&linked, &["twice".into(), "twice".into()]).is_err());

    let bytes = pipeline::emit_wasm(&linked, Some(&exports)).unwrap();
    let glue = wasm_web::javascript(&pipeline::wasm_abi(&linked, &exports).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("hibana-pipeline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wasm = dir.join("math.wasm");
    let js = dir.join("math.mjs");
    std::fs::write(&wasm, bytes).unwrap();
    std::fs::write(&js, glue).unwrap();
    let script = r#"
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';
const { instantiate } = await import(pathToFileURL(process.argv[2]));
const printed = [];
const x = await instantiate(fs.readFileSync(process.argv[1]), { hibana: { print: n => printed.push(n) } });
if (Object.keys(x).join(',') !== 'twice,log' || x.twice(21) !== 42) throw Error('unexpected exports');
x.log(-5);
if (printed[0] !== '-5') throw Error('print');
for (const value of [1.5, Number.MAX_SAFE_INTEGER + 1, 1n, '1']) {
  try { x.twice(value); throw Error('accepted invalid input'); } catch (e) { if (!(e instanceof TypeError || e instanceof RangeError)) throw e; }
}
try { x.twice(); throw Error('accepted missing argument'); } catch (e) { if (!(e instanceof TypeError)) throw e; }
"#;
    let output = Command::new("node")
        .args(["--input-type=module", "-e", script])
        .arg(&wasm)
        .arg(&js)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn glue_preserves_boundary_signatures_and_order() {
    let source = r#"
extern 'host' {
    fn show(message: String)
    fn decorate(message: String, n: int) -> String
    fn count() -> int
    fn unused_host() -> int
}
fn greet(message: String, n: int) -> String { decorate(message, n) }
fn log(message: String) { show(message) }
fn answer() -> int { count() }
fn unused() -> int { unused_host() }
"#;
    // 宣言順とは違う順序で公開し、外部関数は宣言順のままであることを確かめる。
    let exports = vec!["answer".into(), "log".into(), "greet".into()];
    let checked = pipeline::check(pipeline::parse(source).unwrap(), Kind::Library).unwrap();
    let ir = pipeline::link(
        &pipeline::lower(&checked, Some(&exports)).unwrap(),
        &exports,
    )
    .unwrap();
    let glue = wasm_web::javascript(&pipeline::wasm_abi(&ir, &exports).unwrap()).unwrap();
    // 生成した接続コードの順序と変換は、下で WASM を実行して確かめる。

    let dir = std::env::temp_dir().join(format!("hibana-signatures-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wasm = dir.join("signatures.wasm");
    let js = dir.join("signatures.mjs");
    std::fs::write(&wasm, pipeline::emit_wasm(&ir, Some(&exports)).unwrap()).unwrap();
    std::fs::write(&js, glue).unwrap();
    let script = r#"
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';
const { instantiate } = await import(pathToFileURL(process.argv[2]));
const messages = [];
const x = await instantiate(fs.readFileSync(process.argv[1]), { hibana_host: {
  show: message => { messages.push(message); },
  decorate: (message, n) => {
    if (typeof n !== 'number') throw Error('host int');
    return `${message}:${n}`;
  },
  count: () => 42
} });
if (Object.keys(x).join(',') !== 'answer,log,greet') throw Error('export order');
if (x.answer() !== 42) throw Error('int result');
if (x.greet('こんにちは', 7) !== 'こんにちは:7') throw Error('str result');
if (x.log('記録') !== undefined || messages.join(',') !== '記録') throw Error('unit result');
"#;
    let output = Command::new("node")
        .args(["--input-type=module", "-e", script])
        .arg(&wasm)
        .arg(&js)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn load_uses_explicit_base_directory_without_changing_caller() {
    let dir = std::env::temp_dir().join(format!("hibana-load-{}", std::process::id()));
    let a = dir.join("a");
    let b = dir.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(a.join("util.ts"), "export function value(): number { return 1; }").unwrap();
    std::fs::write(b.join("util.ts"), "export function value(): string { return 'two'; }").unwrap();
    let source = "use './util.ts'\n";
    hibana::parser::set_base_dir(&a);
    let loaded = pipeline::load(source, &b).unwrap();
    assert!(loaded.modules.iter().any(|module| module.source.text.contains("-> String")));
    let loaded = hibana::parser::load_modules(source, &a).unwrap();
    assert!(loaded.modules.iter().any(|module| module.source.text.contains("-> number")));
    std::fs::write(a.join("util.hb"), "fn value() -> int { 1 }").unwrap();
    std::fs::write(b.join("util.hb"), "fn value() -> int { 2 }").unwrap();
    let loaded = pipeline::load("use util", &b).unwrap();
    assert!(loaded.modules.iter().any(|module| module.source.text.contains("{ 2 }")));
    let loaded = hibana::parser::load_modules("use util", &a).unwrap();
    assert!(loaded.modules.iter().any(|module| module.source.text.contains("{ 1 }")));
    std::fs::remove_dir_all(dir).unwrap();
}
