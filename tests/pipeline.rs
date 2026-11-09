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
