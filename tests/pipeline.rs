use hibana::pipeline::{self, Kind};
use std::path::Path;
use std::process::Command;

#[test]
fn builds_and_links_as_independent_steps() {
    let src = "fn add(a: int, b: int) -> int { a + b }\nfn twice(n: int) -> int { add(n, n) }\nfn log(n: int) { print(n) }\nfn unused(n: int) -> int { n + 999 }";
    let expanded = pipeline::load(src, Path::new(".")).unwrap();
    let parsed = pipeline::parse(&expanded).unwrap();
    let checked = pipeline::check(parsed, Kind::Library).unwrap();
    assert_eq!(checked.program().fns.len(), 4);
    let all = pipeline::lower(&checked, None).unwrap();
    assert_eq!(all.functions().len(), 4);
    let exports = vec!["twice".to_string(), "log".to_string()];
    let linked = pipeline::link(&all, &exports).unwrap();
    assert_eq!(
        linked
            .functions()
            .iter()
            .map(|f| f.name())
            .collect::<Vec<_>>(),
        ["add", "twice", "log"]
    );
    assert_eq!(linked.functions()[1].params(), 1);
    assert!(!linked.functions()[1].instructions().is_empty());
    assert!(pipeline::link(&all, &["missing".into()]).is_err());
    assert!(pipeline::emit_wasm(&linked, Some(&["unused".into()])).is_err());
    assert!(pipeline::emit_js_glue(&linked, &["twice".into(), "twice".into()]).is_err());

    let bytes = pipeline::emit_wasm(&linked, Some(&exports)).unwrap();
    let glue = pipeline::emit_js_glue(&linked, &exports).unwrap();
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
if (Object.keys(x).join(',') !== 'twice,log' || x.twice(21) !== 42n) throw Error('unexpected exports');
x.log(-5n);
if (printed[0] !== -5n) throw Error('print');
for (const value of [1.5, Number.MAX_SAFE_INTEGER + 1, 1n << 63n, '1']) {
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
fn inspection_and_glue_preserve_boundary_signatures_and_order() {
    let source = r#"
extern "host" {
    fn show(message: str)
    fn decorate(message: str, n: int) -> str
    fn count() -> int
    fn unused_host() -> int
}
fn greet(message: str, n: int) -> str { decorate(message, n) }
fn log(message: str) { show(message) }
fn answer() -> int { count() }
fn unused() -> int { unused_host() }
"#;
    // 宣言順とは違う順序で公開し、外部関数は宣言順のままであることを確かめる。
    let exports = vec!["answer".into(), "log".into(), "greet".into()];
    let inspected: serde_json::Value =
        serde_json::from_str(&pipeline::inspect_exports(source, &exports).unwrap()).unwrap();
    assert_eq!(
        inspected,
        serde_json::json!({
            "exports": [
                ["answer", [], "int"],
                ["log", ["str"], "unit"],
                ["greet", ["str", "int"], "str"]
            ],
            "imports": [
                ["show", ["str"], "unit"],
                ["decorate", ["str", "int"], "str"],
                ["count", [], "int"]
            ]
        })
    );
    let checked = pipeline::check(pipeline::parse(source).unwrap(), Kind::Library).unwrap();
    let ir = pipeline::link(
        &pipeline::lower(&checked, Some(&exports)).unwrap(),
        &exports,
    )
    .unwrap();
    let glue = pipeline::emit_js_glue(&ir, &exports).unwrap();
    assert!(glue.contains(&format!(
        "for (const [name, params, result] of {})",
        inspected["exports"]
    )));
    assert!(glue.contains(&format!(
        "for (const [name, params, result] of {})",
        inspected["imports"]
    )));

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
    if (typeof n !== 'bigint') throw Error('host int');
    return `${message}:${n}`;
  },
  count: () => 42n
} });
if (Object.keys(x).join(',') !== 'answer,log,greet') throw Error('export order');
if (x.answer() !== 42n) throw Error('int result');
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
    std::fs::write(a.join("util.hb"), "fn value() -> int { 1 }").unwrap();
    std::fs::write(b.join("util.hb"), "fn value() -> int { 2 }").unwrap();
    let source = "use \"util.hb\"\n";
    hibana::parser::set_base_dir(&a);
    assert!(pipeline::load(source, &b).unwrap().contains("{ 2 }"));
    assert!(hibana::parser::with_uses(source).unwrap().contains("{ 1 }"));
    std::fs::remove_dir_all(dir).unwrap();
}
