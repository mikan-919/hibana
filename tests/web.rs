use hibana::pipeline::{self, Kind};
use std::process::Command;

#[test]
fn reserved_wasm_exports_are_rejected() {
    for name in ["memory", "__hibana_alloc", "__hibana_reset", "dispose"] {
        let source = format!("fn {name}(n: int) -> int {{ n }}");
        let exports = [name.to_string()];
        assert!(hibana::to_web(&source, &exports).is_err());
        let checked = pipeline::check(pipeline::parse(&source).unwrap(), Kind::Library).unwrap();
        let ir = pipeline::lower(&checked, None).unwrap();
        assert!(pipeline::emit_js_glue(&ir, &exports).is_err());
    }
}

#[test]
fn web_strings_and_host_functions() {
    let source = r#"
extern 'browser' {
    fn decorate(s: String) -> String
    fn show(s: String)
    fn increment(n: int) -> int
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
fn unused(s: String) -> String { s }
"#;
    let checked = pipeline::check(pipeline::parse(source).unwrap(), Kind::Library).unwrap();
    let all = pipeline::lower(&checked, None).unwrap();
    let exports = ["echo", "literal", "run", "add", "notify"].map(String::from);
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
  increment: n => n + 1n,
  __EXPORTS__: s => s,
}});
assert.deepEqual(Object.keys(api), ['echo', 'literal', 'run', 'add', 'notify']);
for (const text of ['', '日本語🌸', 'a\0b', 'x'.repeat(200000)]) {
  assert.equal(api.echo(text), text);
  assert.equal(api.run(text), '[' + text + ']');
  assert.equal(messages.at(-1), '[' + text + ']');
}
assert.equal(api.literal(), 'こんにちは🌸');
assert.equal(api.add(41), 42n);
assert.equal(api.notify('done'), undefined);
assert.equal(messages.at(-1), 'done');
assert.throws(() => api.echo(123), TypeError);
assert.throws(() => api.echo(), TypeError);
assert.throws(() => api.add(1.5), RangeError);
assert.throws(() => api.add(1n << 63n), RangeError);
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

#[test]
fn wasm_opaque_references_preserve_state_and_check_owner_and_type() {
    let source = r#"
        type Positive = int[n -> n > 0]
        fn cell() -> Ref<int> { new_ref(0) }
        fn positive() -> Ref<Positive> { new_ref(1) }
        fn text() -> Ref<String> { new_ref('value') }
        fn increment(cell: Ref<int>, n: int) -> int { write(cell, read(cell) + n); read(cell) }
        fn read_positive(cell: Ref<Positive>) -> Positive { read(cell) }
        fn get(cell: Ref<String>) -> String { read(cell) }
        fn invert(value: bool) -> bool { !value }
    "#;
    let exports = [
        "cell",
        "positive",
        "text",
        "increment",
        "read_positive",
        "get",
        "invert",
    ]
    .map(String::from);
    let checked = pipeline::check(pipeline::parse(source).unwrap(), Kind::Library).unwrap();
    let ir = pipeline::lower(&checked, Some(&exports)).unwrap();
    let dir = std::env::temp_dir().join(format!("hibana-references-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("app.wasm"),
        pipeline::emit_wasm(&ir, Some(&exports)).unwrap(),
    )
    .unwrap();
    std::fs::write(
        dir.join("app.mjs"),
        pipeline::emit_js_glue(&ir, &exports).unwrap(),
    )
    .unwrap();
    let script = r#"
        import assert from 'node:assert/strict';
        import fs from 'node:fs';
        import { pathToFileURL } from 'node:url';
        const { instantiate } = await import(pathToFileURL(process.argv[2]));
        const bytes = fs.readFileSync(process.argv[1]);
        const api = await instantiate(bytes), other = await instantiate(bytes);
        const cell = api.cell(), text = api.text(), positive = api.positive();
        for (let i = 1; i <= 100; i++) {
          assert.equal(api.increment(cell, 1), BigInt(i));
          assert.equal(api.get(text), 'value');
        }
        assert.equal(api.read_positive(positive), 1n);
        assert.throws(() => api.read_positive(cell), TypeError);
        assert.throws(() => api.get(cell), TypeError);
        assert.throws(() => other.increment(cell, 1), TypeError);
        assert.throws(() => api.increment(1n, 1), TypeError);
        assert.equal(api.invert(false), true);
        assert.equal(api.invert(true), false);
        assert.throws(() => api.invert(0), TypeError);
        api.dispose();
        assert.throws(() => api.increment(cell, 1), TypeError);
        assert.equal(api.increment(api.cell(), 2), 2n);
    "#;
    let output = Command::new("node")
        .args(["--input-type=module", "-e", script])
        .arg(dir.join("app.wasm"))
        .arg(dir.join("app.mjs"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
