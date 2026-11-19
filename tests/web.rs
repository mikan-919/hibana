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
fn web_async_exports_return_completed_values() {
    let source = r#"
fn step(n: int) -> int with Async { n + 1 }
fn announce() with Async { put(step(9).await) }
pub fn twice(n: int) -> int with Async {
    if n == 0 {
        spawn(announce())
        return 1 % n
    }
    if n == 1 {
        task_then(spawn(step(n)), fn(value) { panic('then の失敗 ' + value.to_string()) })
    }
    let first = step(n).await
    step(first).await
}
pub fn main(label: String, n: int) -> String with Async {
    let value = twice(n).await
    label + ': ' + value.to_string()
}
"#;
    let exports = ["twice", "main"].map(String::from);
    let web = hibana::to_web(source, &exports).unwrap();
    let checked = pipeline::check(pipeline::parse(source).unwrap(), Kind::Library).unwrap();
    let all = pipeline::lower(&checked, None).unwrap();
    let linked_exports = ["twice".into()];
    let linked = pipeline::link(&all, &linked_exports).unwrap();
    let dir = std::env::temp_dir().join(format!("hibana-web-async-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("web.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("web.mjs"), web.javascript).unwrap();
    std::fs::write(dir.join("linked.wasm"), pipeline::emit_wasm(&linked, Some(&linked_exports)).unwrap()).unwrap();
    std::fs::write(dir.join("linked.mjs"), pipeline::emit_js_glue(&linked, &linked_exports).unwrap()).unwrap();
    let script = r#"
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
for (const name of ['web', 'linked']) {
  const { instantiate } = await import(`./${name}.mjs`);
  const messages = [];
  const api = await instantiate(readFileSync(`${name}.wasm`), {
    hibana: { print: message => { messages.push(message); } },
  });
  assert.equal(api.twice(40), 42);
  if (name === 'web') assert.equal(api.main('結果', 8), '結果: 10');
  assert.throws(() => api.twice(0), { name: 'RuntimeError', message: '実行時エラー: 0 で除算した' });
  assert.throws(() => api.twice(1), { name: 'RuntimeError', message: '実行時エラー: then の失敗 2' });
  assert.equal(api.twice(-2), 0);
  assert.deepEqual(messages, ['10']);
}
"#;
    let output = Command::new("node")
        .args(["--input-type=module", "-e", script])
        .current_dir(&dir)
        .output()
        .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn web_sync_main_drains_started_tasks_before_throwing() {
    let source = r#"
fn step(n: int) -> int with Async { n }
fn work() with Async { put(step(42).await) }
pub fn main(fail: bool) with {} {
    spawn(work())
    if fail { panic('main の失敗') }
}
"#;
    let web = hibana::to_web(source, &["main".into()]).unwrap();
    let dir = std::env::temp_dir().join(format!("hibana-web-sync-main-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("web.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("web.mjs"), web.javascript).unwrap();
    let script = r#"
import { readFileSync } from 'node:fs';
import assert from 'node:assert/strict';
import { instantiate } from './web.mjs';
const messages = [];
const api = await instantiate(readFileSync('web.wasm'), {
  hibana: { print: message => { messages.push(message); } },
});
assert.throws(() => api.main(true), { name: 'RuntimeError', message: '実行時エラー: main の失敗' });
assert.deepEqual(messages, ['42']);
api.main(false);
assert.deepEqual(messages, ['42', '42']);
"#;
    let output = Command::new("node")
        .args(["--input-type=module", "-e", script])
        .current_dir(&dir)
        .output()
        .unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
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

// Vec と構造体（入れ子を含む）を、公開関数と外部関数の引数・戻り値に渡せる。JavaScript では配列とオブジェクト
#[test]
fn web_vectors_and_structs() {
    let source = r#"
struct Point { x: int, y: number }
struct Line { name: String, points: Vec<Point>, closed: bool }
extern 'browser' {
    fn scale(ps: Vec<Point>, k: int) -> Vec<Point>
    fn label(l: Line) -> String
}
fn sum(xs: Vec<int>) -> int {
    let mut s = 0
    for x in xs { s = s + x }
    s
}
fn words(s: String) -> Vec<String> { split(s, ' ') }
fn shout(xs: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = []
    for x in xs { out = push(out, x + '!') }
    out
}
fn origin() -> Point { Point { x: 0, y: 0.5 } }
fn flip(p: Point) -> Point { Point { x: p.x * 2, y: p.y + 1.0 } }
fn line(name: String, ps: Vec<Point>) -> Line { Line { name: name, points: scale(ps, 3), closed: len(ps) > 2 } }
fn describe(l: Line) -> String { label(l) }
fn nested(xs: Vec<Vec<int>>) -> Vec<int> {
    let mut out: Vec<int> = []
    for x in xs { out = push(out, sum(x)) }
    out
}
"#;
    let exports = ["sum", "words", "shout", "origin", "flip", "line", "describe", "nested"].map(String::from);
    let web = hibana::to_web(source, &exports).unwrap();
    let dir = std::env::temp_dir().join(format!("hibana-web-values-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wasm_path = dir.join("web.wasm");
    let glue_path = dir.join("web.mjs");
    std::fs::write(&wasm_path, web.wasm).unwrap();
    std::fs::write(&glue_path, web.javascript).unwrap();
    let script = r#"
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';
const { instantiate } = await import(pathToFileURL(process.argv[2]));
const api = await instantiate(fs.readFileSync(process.argv[1]), { hibana_host: {
  scale: (ps, k) => ps.map((p) => ({ x: p.x * k, y: p.y * k })),
  label: (l) => l.name + ':' + l.points.length + ':' + l.closed,
}});
assert.equal(api.sum([1, 2, 3, -4]), 2);
assert.equal(api.sum([]), 0);
assert.deepEqual(api.words('日本 の 🌸'), ['日本', 'の', '🌸']);
assert.deepEqual(api.shout(['a', '', 'é']), ['a!', '!', 'é!']);
assert.deepEqual(api.origin(), { x: 0, y: 0.5 });
assert.deepEqual(api.flip({ x: 3, y: 1.5 }), { x: 6, y: 2.5 });
assert.deepEqual(api.flip({ x: 3, y: 2 }), { x: 6, y: 3 });
assert.deepEqual(api.line('l', [{ x: 1, y: 1 }, { x: 2, y: 0.5 }]), { name: 'l', points: [{ x: 3, y: 3 }, { x: 6, y: 1.5 }], closed: false });
assert.equal(api.describe({ name: 'tri', points: [{ x: 0, y: 0 }, { x: 1, y: 0 }, { x: 0, y: 1 }], closed: true }), 'tri:3:true');
assert.deepEqual(api.nested([[1, 2], [], [3]]), [3, 0, 3]);
const many = Array.from({ length: 5000 }, (_, i) => i);
assert.equal(api.sum(many), 5000 * 4999 / 2);
assert.throws(() => api.sum([1, 'x']), RangeError);
assert.throws(() => api.sum('x'), TypeError);
assert.throws(() => api.flip({ x: 1 }), TypeError);
assert.throws(() => api.flip({ x: 1, y: Infinity }), TypeError);
"#;
    let output = Command::new("node")
        .args(["--input-type=module", "-e", script])
        .arg(&wasm_path)
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn host_return_types_match_in_javascript_and_wasm() {
    let source = r#"
struct Value { labels: Vec<String>, enabled: bool }
extern '' {
    fn flag() -> bool
    fn label() -> String
    fn value() -> Value
}
fn main() {
    put(flag())
    put(label())
    let v = value()
    put(v.labels)
    put(v.enabled)
}
"#;
    let js = hibana::to_js(source).unwrap();
    let web = hibana::to_web(source, &["main".into()]).unwrap();
    let dir = std::env::temp_dir().join(format!("hibana-web-host-types-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("web.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("web.mjs"), web.javascript).unwrap();
    let web_runner = r#"
import { readFileSync } from 'node:fs';
import { instantiate } from './web.mjs';
const api = await instantiate(readFileSync('web.wasm'), { hibana_host: { flag, label, value } });
try { api.main(); } catch (e) { console.error(e.message); process.exitCode = 1; }
"#;
    for (flag, label, value, expected, error) in [
        ("false", "'ねこ'", "{ labels: ['a', ''], enabled: true }", "false\nねこ\n[\"a\", \"\"]\ntrue\n", None),
        ("1", "'ねこ'", "null", "", Some("真偽値が必要")),
        ("false", "7", "null", "false\n", Some("文字列が必要")),
        ("false", "'ねこ'", "{ labels: ['a', 7], enabled: true }", "false\nねこ\n", Some("文字列が必要")),
        ("false", "'ねこ'", "{ labels: [], enabled: 'true' }", "false\nねこ\n", Some("真偽値が必要")),
        ("false", "'ねこ'", "{ labels: 'a', enabled: true }", "false\nねこ\n", Some("配列が必要")),
    ] {
        let host = format!("globalThis.flag = () => {flag}; globalThis.label = () => {label}; globalThis.value = () => ({value});");
        for runner in [&js, web_runner] {
            let output = Command::new("node")
                .args(["--input-type=module", "-e", &format!("{host}\n{runner}")])
                .current_dir(&dir)
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(output.status.success(), error.is_none(), "{host}: {stderr}");
            assert_eq!(output.stdout, expected.as_bytes(), "{host}");
            if let Some(error) = error {
                assert!(stderr.contains(error), "{host}: {stderr}");
            } else {
                assert!(stderr.is_empty(), "{host}: {stderr}");
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn javascript_checks_callback_arguments_and_async_host_results() {
    let source = r#"
type Label = String
struct Payload { enabled: bool, labels: Vec<Label> }
extern '' {
    fn invoke(f: (Payload) -> String) -> String
    fn invoke_async(f: (bool) -> String with Async) -> String with Async
}
fn label(p: Payload) -> String { if p.enabled && len(p.labels) > 0 { p.labels[0] } else { 'off' } }
fn status(ok: bool) -> String with Async { if ok { 'on' } else { 'off' } }
fn main() with Async {
    put(invoke(label))
    put(invoke_async(status).await)
}
"#;
    let js = hibana::to_js(source).unwrap();
    for (payload, async_call, expected, error) in [
        ("{ enabled: true, labels: ['ok'] }", "f(false)", "ok\noff\n", None),
        ("{ enabled: 1, labels: [] }", "f(false)", "", Some("真偽値が必要")),
        ("{ enabled: false, labels: [7] }", "f(false)", "", Some("文字列が必要")),
        ("{ enabled: true, labels: ['ok'] }", "f(1)", "ok\n", Some("真偽値が必要")),
        ("{ enabled: true, labels: ['ok'] }", "7", "ok\n", Some("文字列が必要")),
    ] {
        let host = format!("globalThis.invoke = f => f({payload}, '余分な引数'); globalThis.invoke_async = async f => {async_call};");
        let output = Command::new("node")
            .args(["-e", &format!("{host}\n{js}")])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.success(), error.is_none(), "{host}: {stderr}");
        assert_eq!(output.stdout, expected.as_bytes(), "{host}");
        if let Some(error) = error {
            assert!(stderr.contains(error), "{host}: {stderr}");
        } else {
            assert!(stderr.is_empty(), "{host}: {stderr}");
        }
    }
}
