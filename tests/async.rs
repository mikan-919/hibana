#[path = "common/wasm_web.rs"]
mod wasm_web;

use std::process::Command;

#[test]
fn async_annotations_survive_cloning_and_numeric_elaboration() {
    let source = r#"
extern '' { fn delay(n: int) -> int with Async }
fn apply<E: Effect>(f: () -> int with E) -> int with E { f() }
fn main() with Async {
    let result: number = apply(fn() { delay(7).await })
    put(result)
    put(all([delay(2), delay(3)]).await)
}
"#;
    let mut program = hibana::parser::parse(source).unwrap();
    let info = hibana::typeck::check(&mut program).unwrap();
    let mut moved = program.clone();
    moved.fns.reserve(100);
    drop(program);
    let javascript = hibana::js::emit(&moved, &info);
    let output = Command::new("node")
        .args([
            "-e",
            &format!("globalThis.delay = async n => n;\n{javascript}"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"7\n[2, 3]\n");
}

// JavaScript の spawn は Promise を走らせ始める。peek は終わるまで None、then は終わった時に呼ぶ。失敗は console.error に出す
#[test]
fn spawned_tasks_run_concurrently_in_javascript() {
    let source = r#"
extern '' { fn delay(n: int) -> int with Async }
extern '' { fn boom() -> int with Async }
fn main() with Async {
    let t = spawn(delay(1))
    put(t.peek())
    t.then(fn(v) { put('then ' + v.to_string()) })
    spawn(boom())
    put(delay(30).await)
    put(t.peek())
}
"#;
    let javascript = hibana::to_js(source).unwrap();
    let output = Command::new("node")
        .args([
            "-e",
            &format!(
                "globalThis.delay = n => new Promise((r) => setTimeout(() => r(n), n));\nglobalThis.boom = async () => {{ throw new Error('boom'); }};\n{javascript}"
            ),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "None\nthen 1\n30\nSome(1)\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("実行時エラー: boom"));
}

fn javascript_error(source: &str, host: &str) -> std::process::Output {
    let javascript = hibana::to_js(source).unwrap();
    Command::new("node")
        .args(["-e", &format!("{host}\n{javascript}")])
        .output()
        .unwrap()
}

#[test]
fn concurrent_await_errors_keep_the_failing_expression_position() {
    let source = "extern '' { fn boom() -> int with Async }\n\
extern '' { fn pause() -> int with Async }\n\
fn noise() -> int with Async {\n    pause().await\n    1 + 1\n}\n\
fn main() with Async {\n    put(all([boom(), noise()]).await)\n}\n";
    let output = javascript_error(
        source,
        "globalThis.boom = async () => { throw new Error('boom'); };\n\
globalThis.pause = async () => 1;",
    );
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "実行時エラー: boom（8 行 14 列）\n"
    );
}

#[test]
fn spawned_errors_keep_the_failing_expression_position() {
    let source = "extern '' { fn boom() -> int with Async }\n\
extern '' { fn pause() -> int with Async }\n\
fn main() with Async {\n    spawn(boom())\n    put(pause().await + 1)\n}\n";
    for boom in [
        "() => { throw new Error('boom'); }",
        "async () => { throw new Error('boom'); }",
    ] {
        let output = javascript_error(
            source,
            &format!("globalThis.boom = {boom}; globalThis.pause = async () => 0;"),
        );
        assert!(output.status.success(), "{:?}", output);
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "実行時エラー: boom（4 行 11 列）\n"
        );
    }
}

#[test]
fn arithmetic_errors_keep_the_operator_position_after_evaluating_operands() {
    for operand in ["(2 - 1)", "pause().await"] {
        let source = format!(
            "extern '' {{ fn pause() -> int with Async }}\n\
fn main() with Async {{\n    put(9007199254740991 + {operand})\n}}\n"
        );
        let output = javascript_error(&source, "globalThis.pause = async () => 1;");
        assert!(!output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "実行時エラー: オーバーフロー（3 行 9 列）\n"
        );
    }
}


#[test]
fn runtime_integer_refinement_evaluates_an_async_argument_once() {
    let source = "use std -> *\nextern '' { fn once() -> number with Async }\nfn main() with Async { put(refine<int>(once().await)) }";
    let javascript = hibana::to_js(source).unwrap();
    let output = Command::new("node").args(["-e", &format!("let calls = 0; globalThis.once = async () => {{ if (++calls > 1) throw Error('呼び出しが重複'); return 3; }};\n{javascript}")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"std.Ok(3)\n");
}

// WASM とネイティブの非同期は、JavaScript の Promise とマイクロタスクと同じ順で動く（src/backend/ir/coroutine.rs）。
fn ir_matches_javascript(name: &str, source: &str, panic: Option<&str>) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("hibana-async-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let js = dir.join("main.cjs");
    std::fs::write(&js, hibana::to_js(source).unwrap()).unwrap();
    let want = Command::new("node").arg(&js).output().unwrap();
    assert!(want.status.success(), "{}", String::from_utf8_lossy(&want.stderr));
    assert_panic_report(&want.stderr, panic, "JavaScript");
    let web = wasm_web::build(source, &["main".into()]).unwrap();
    std::fs::write(dir.join("main.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let runner = dir.join("run.mjs");
    std::fs::write(&runner, "import {readFileSync} from 'node:fs'; import {instantiate} from './glue.mjs'; const api = await instantiate(readFileSync(new URL('./main.wasm', import.meta.url))); api.main();").unwrap();
    let wasm = Command::new("node").arg(&runner).output().unwrap();
    assert!(wasm.status.success(), "{}", String::from_utf8_lossy(&wasm.stderr));
    assert_eq!(String::from_utf8_lossy(&wasm.stdout), String::from_utf8_lossy(&want.stdout), "WASM: {name}");
    assert_panic_report(&wasm.stderr, panic, "WASM");
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.join("main");
        std::fs::write(&exe, hibana::to_native(source).unwrap()).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let native = Command::new(&exe).output().unwrap();
        assert!(native.status.success(), "{}", String::from_utf8_lossy(&native.stderr));
        assert_eq!(String::from_utf8_lossy(&native.stdout), String::from_utf8_lossy(&want.stdout), "ネイティブ: {name}");
        assert_panic_report(&native.stderr, panic, "ネイティブ");
    }
    std::fs::remove_dir_all(dir).unwrap();
    want.stdout
}

fn assert_panic_report(stderr: &[u8], panic: Option<&str>, backend: &str) {
    let stderr = String::from_utf8_lossy(stderr);
    let lines: Vec<_> = stderr.lines().collect();
    if let Some(message) = panic {
        assert_eq!(lines.len(), 1, "{backend}: {stderr}");
        let expected = format!("実行時エラー: {message}");
        assert_eq!(lines[0].split('（').next(), Some(expected.as_str()), "{backend}");
    } else {
        assert!(lines.is_empty(), "{backend}: {stderr}");
    }
}

fn all_backends_match_javascript(name: &str, source: &str) {
    let want = ir_matches_javascript(name, source, None);
    let mut output = Vec::new();
    hibana::run(source, &mut output).unwrap();
    assert_eq!(String::from_utf8_lossy(&output), String::from_utf8_lossy(&want), "インタプリタ: {name}");
}

#[test]
fn spawned_panic_reports_once_and_other_tasks_continue() {
    let source = r#"
use std -> *

fn step() -> int with Async { 42 }
fn fail_in_helper() -> int { panic('タスクの失敗') }
fn fail_later() -> int with Async {
    step().await
    fail_in_helper()
}
trait Answer {
    fn answer(self) -> int
}
fn bad() -> int with Async {
    let n = with Answer(fn() { 1 }) {
        fail_later().await + answer()
    }
    put('失敗後の本体')
    n
}
fn good() -> int with Async { step().await }

fn main() {
    spawn(bad()).then(fn(n) { put(n) })
    spawn(good()).then(fn(n) { put(n) })
    put('main')
}
"#;
    let want = ir_matches_javascript("panic", source, Some("タスクの失敗"));
    assert_eq!(want, b"main\n42\n");

    let path = std::env::temp_dir().join(format!("hibana-async-panic-{}.hb", std::process::id()));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_hibana")).arg(&path).output().unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, want);
    assert_panic_report(&output.stderr, Some("タスクの失敗"), "インタプリタ");
}

#[test]
fn spawn_runs_to_the_first_await_before_returning() {
    all_backends_match_javascript("spawn", r#"
use std -> *

fn step() -> int with Async { 42 }

fn work() -> int with Async {
    put('before await')
    let value = step().await
    put('after await')
    value
}

fn main() {
    let t = spawn(work())
    put(t.peek())
    t.then(fn(v) { put(v) })
    put('main end')
}
"#);
}

// spawn した処理・then・all・要求の async な腕・with を抜ける失敗が、JavaScript と同じ順で出力する
#[test]
fn ir_async_order_matches_javascript() {
    all_backends_match_javascript("order", r#"
use std -> *

fn step(name: String, n: int) -> int with Async {
    put(name + ' ' + n.to_string())
    n
}

fn chain(name: String) -> int with Async {
    let a = step(name, 1).await
    let b = step(name, 2).await
    a + b
}

trait Fetch {
    fn fetch(self, k: int) -> int with Async
}

fn use_fetch() -> int with Async, Fetch {
    fetch(1).await + fetch(2).await
}

fn main() with Async {
    spawn(chain('a')).then(fn(v) { put('then a ' + v.to_string()) })
    let t = spawn(chain('b'))
    put('main 1')
    put(t.peek())
    put(step('main', 3).await)
    put(all([chain('c'), chain('d')]).await)
    put(t.peek())
    put(map([1, 2], fn(x) { step('m', x).await * 10 }))
    let r = with Fetch(fn(k) { step('f', k).await }) {
        use_fetch().await
    }
    put(r)
    let e = with Fail(fn(m) { exit m }) {
        all([ok_later(), fail_later()]).await.to_string()
    }
    put(e)
    put('main end')
}

fn fail_later() -> int with Async, Fail<String> {
    step('y', 1).await
    fail('boom')
}

fn ok_later() -> int with Async, Fail<String> {
    step('x', 1).await
}
"#);
}

// await を含むループ・分岐・早い return・再帰・入れ子の with を抜ける中断・要求の多相な関数の async 版・join
#[test]
fn ir_async_control_flow_matches_javascript() {
    all_backends_match_javascript("control", r#"
use std -> *

fn tick(name: String, n: int) -> int with Async {
    put(name + n.to_string())
    n
}

fn loop_sum(name: String, n: int) -> int with Async {
    let mut s = 0
    let mut i = 0
    while i < n {
        i = i + 1
        if i == 2 { continue }
        if i == 5 { break }
        s = s + tick(name, i).await
    }
    for x in [10, 20] {
        if x > 15 { s = s + tick(name, x).await } else { s = s - tick(name, x).await }
    }
    s
}

fn early(n: int) -> int with Async {
    let mut i = 0
    while true {
        i = i + tick('e', i).await
        if i > n { return i * 100 }
        i = i + 1
    }
    0
}

fn fib(n: int) -> int with Async {
    if n < 2 { tick('f', n).await } else { fib(n - 1).await + fib(n - 2).await }
}

trait Fail2 {
    fn bail(self, m: String) -> int
}

fn risky(n: int) -> int with Async, Fail2 {
    let a = tick('r', n).await
    if a > 1 { bail('too big ' + a.to_string()) } else { a }
}

fn guarded(n: int) -> String with Async {
    with Fail2(fn(m) { exit m }) {
        let v = risky(n).await
        'ok ' + v.to_string()
    }
}

fn nested_exit() -> int with Async {
    let outer = with Fail2(fn(m) { exit 1000 }) {
        let inner = with Fail2(fn(m) { exit len(m) }) {
            tick('n', 1).await
            bail('xyz')
        }
        tick('n', inner).await + bail('q')
    }
    outer
}

fn main() with Async {
    let t1 = spawn(loop_sum('a', 10))
    let t2 = spawn(early(3))
    t1.then(fn(v) { put('t1 ' + v.to_string()) })
    put('start')
    put(fib(4).await)
    put(guarded(1).await)
    put(guarded(5).await)
    put(nested_exit().await)
    put(filter([1, 2, 3, 4], fn(x) { tick('p', x).await % 2 == 0 }))
    put(fold([1, 2, 3], 0, fn(acc, x) { acc + tick('s', x).await }))
    let p = join(tick('j', 1), tick('k', 2)).await
    put(p.first + p.second)
    put(all([loop_sum('b', 3), early(1), fib(2)]).await)
    t2.then(fn(v) { put('t2 ' + v.to_string()) })
    spawn(tick('z', 9)).then(fn(v) {
        spawn(tick('w', v + 1)).then(fn(u) { put('inner ' + u.to_string()) })
    })
    put(t1.peek())
    let mut total = 0
    for i in 0..3 {
        total = total + len(all([tick('l', i), tick('m', i * 2)]).await)
    }
    put(total)
    put('end')
}
"#);
}

// 待っている間のコルーチンの区画は、Promise の反応と仕事の並びから辿れるので GC に回収されない
#[test]
fn ir_async_survives_gc() {
    ir_matches_javascript("gc", &r#"
use std -> *

fn piece(i: int) -> String with Async {
    'p' + i.to_string() + [i, i].to_string()
}

fn work(i: int) -> int with Async {
    let mut s = ''
    for k in 0..5 {
        s = s + piece(i * 10 + k).await
    }
    len(s)
}

fn main() with Async {
    let total = new_ref(0)
    let mut tasks: Vec<Task<int>> = []
    for i in 0..3000 {
        let t = spawn(work(i))
        t.then(fn(v) { write(total, read(total) + v) })
        tasks = push(tasks, t)
    }
    let mut sum = 0
    for i in 0..200 {
        sum = sum + len(all([work(i), work(i + 1)]).await)
    }
    put(sum)
    put(len(tasks))
    spawn(piece(0)).then(fn(s) { put(read(total)) })
}
"#.replace("0..3000", "0..20000"), None);
}
