use hibana::pipeline::{self, Kind};
use std::process::Command;

#[test]
fn reserved_wasm_exports_are_rejected() {
    for name in ["memory", "__hibana_alloc", "__hibana_reset"] {
        let source = format!("fn {name}(n: int) -> int {{ n }}");
        let exports = [name.to_string()];
        assert!(hibana::to_web(&source, &exports).is_err());
        let checked = pipeline::check(pipeline::parse(&source).unwrap(), Kind::Library).unwrap();
        let ir = pipeline::lower(&checked, None).unwrap();
        assert!(pipeline::emit_js_glue(&ir, &exports).is_err());
    }
}

#[test]
fn references_cannot_cross_the_wasm_boundary() {
    for source in [
        "fn expose(r: Ref<int>) -> int { read(r) }",
        "fn expose() -> Ref<int> { new_ref(1) }",
        "extern 'host' { fn consume(r: Ref<int>) -> int } fn expose() -> int { consume(new_ref(1)) }",
        "extern 'host' { fn produce() -> Ref<int> } fn expose() -> int { read(produce()) }",
    ] {
        assert!(hibana::to_web(source, &["expose".into()]).is_err());
    }
}

#[test]
fn web_strings_and_host_functions() {
    let source = r#"
extern 'browser' {
    fn decorate(s: String) -> String
    fn show(s: String)
    fn increment(n: int) -> int
    fn invert(value: bool) -> bool
    fn unused_host(n: int) -> int
    fn __EXPORTS__(s: String) -> String
}
fn echo(s: String) -> String { __EXPORTS__(s) }
fn literal() -> String { 'こんにちは🌸' }
fn run(s: String) -> String {
    let result = decorate(s)
    show(result)
    result
}
fn add(n: int) -> int { increment(n) }
fn notify(s: String) { show(s) }
fn negate(value: bool) -> bool { invert(value) }
fn dispose() -> int { 7 }
fn unused(s: String) -> String { s }
"#;
    let checked = pipeline::check(pipeline::parse(source).unwrap(), Kind::Library).unwrap();
    let all = pipeline::lower(&checked, None).unwrap();
    let exports = ["echo", "literal", "run", "add", "notify", "negate", "dispose"].map(String::from);
    let linked = pipeline::link(&all, &exports).unwrap();
    assert!(!linked.functions().iter().any(|f| f.name() == "unused"));
    let wasm = pipeline::emit_wasm(&linked, Some(&exports)).unwrap();
    let glue = pipeline::emit_js_glue(&linked, &exports).unwrap();
    let dir = std::env::temp_dir().join(format!("hibana-web-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wasm_path = dir.join("web.wasm");
    let glue_path = dir.join("web.mjs");
    std::fs::write(&wasm_path, wasm).unwrap();
    std::fs::write(&glue_path, glue).unwrap();
    let script = r#"
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';
const { instantiate } = await import(pathToFileURL(process.argv[2]));
const bytes = fs.readFileSync(process.argv[1]);
await assert.rejects(instantiate(bytes), TypeError);
const messages = [];
let mode = 'normal';
let api;
api = await instantiate(bytes, { hibana_host: {
  decorate: s => {
    if (mode === 'throw') throw Error('host failure');
    if (mode === 'invalid') return 123;
    if (mode === 'async') return Promise.resolve(s);
    if (mode === 'reenter') return api.echo(s);
    return '[' + s + ']';
  },
  show: s => { messages.push(s); },
  increment: n => n + 1,
  invert: value => { assert.equal(typeof value, 'boolean'); return !value; },
  __EXPORTS__: s => s,
}});
assert.deepEqual(Object.keys(api), ['echo', 'literal', 'run', 'add', 'notify', 'negate', 'dispose']);
assert.equal(api.dispose(), 7);
assert.equal(api.negate(true), false);
assert.equal(api.negate(false), true);
assert.throws(() => api.negate(1), TypeError);
for (const text of ['', '日本語🌸', 'a\0b', 'x'.repeat(200000)]) {
  assert.equal(api.echo(text), text);
  assert.equal(api.run(text), '[' + text + ']');
  assert.equal(messages.at(-1), '[' + text + ']');
}
assert.equal(api.literal(), 'こんにちは🌸');
assert.equal(api.add(41), 42);
assert.equal(api.notify('done'), undefined);
assert.equal(messages.at(-1), 'done');
assert.throws(() => api.echo(123), TypeError);
assert.throws(() => api.echo(), TypeError);
assert.throws(() => api.add(1.5), RangeError);
assert.throws(() => api.add(1n), RangeError);
for (const next of ['throw', 'invalid', 'async', 'reenter']) {
  mode = next;
  assert.throws(() => api.run('test'));
  mode = 'normal';
  assert.equal(api.run('recovered'), '[recovered]');
}
for (let i = 0; i < 100; i++) assert.equal(api.echo('reuse'), 'reuse');
"#;
    let output = Command::new("node")
        .args(["--input-type=module", "-e", script])
        .arg(&wasm_path)
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
