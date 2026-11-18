use std::fs;
use std::process::Command;

struct Expected<'a> {
    success: bool,
    stdout: &'a str,
    report: Option<&'a str>,
}

fn check_backends(name: &str, source: &str, want: Expected<'_>) {
    let dir = std::env::temp_dir().join(format!("hibana-parity-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();

    let hb = dir.join("main.hb");
    fs::write(&hb, source).unwrap();
    let interpreted = Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg(hb)
        .output()
        .unwrap();

    let javascript = dir.join("main.cjs");
    let js = hibana::to_js(source).unwrap();
    fs::write(&javascript, js).unwrap();
    let js = Command::new("node").arg(javascript).output().unwrap();

    let web = hibana::to_web(source, &["main".into()]).unwrap();
    fs::write(dir.join("main.wasm"), web.wasm).unwrap();
    fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let runner = dir.join("run.mjs");
    fs::write(
        &runner,
        r#"
import {readFileSync} from 'node:fs';
import {instantiate} from './glue.mjs';
const api = await instantiate(readFileSync(new URL('./main.wasm', import.meta.url)));
try { api.main(); } catch (e) { console.error(e.message); process.exitCode = 1; }
"#,
    )
    .unwrap();
    let wasm = Command::new("node").arg(runner).output().unwrap();
    let mut outputs = vec![
        ("インタプリタ", interpreted),
        ("JavaScript", js),
        ("WASM", wasm),
    ];

    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let native = dir.join("main");
        fs::write(&native, hibana::to_native(source).unwrap()).unwrap();
        fs::set_permissions(&native, fs::Permissions::from_mode(0o755)).unwrap();
        outputs.push(("ネイティブ", Command::new(native).output().unwrap()));
    }

    for (backend, output) in outputs {
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert_eq!(
            output.status.success(),
            want.success,
            "{name} / {backend}: {stderr}"
        );
        assert_eq!(
            std::str::from_utf8(&output.stdout).unwrap(),
            want.stdout,
            "{name} / {backend}"
        );
        if let Some(report) = want.report {
            let mut lines = stderr.lines().filter(|line| !line.trim().is_empty());
            assert!(
                lines.next().is_some_and(|line| line.contains(report)),
                "{name} / {backend}: {stderr}"
            );
            assert_eq!(
                lines.filter(|line| line.contains("実行時エラー:")).count(),
                0,
                "{name} / {backend}: {stderr}"
            );
            if backend != "インタプリタ" || want.success {
                assert_eq!(stderr.lines().count(), 1, "{name} / {backend}: {stderr}");
            }
        } else {
            assert!(stderr.is_empty(), "{name} / {backend}: {stderr}");
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

fn success(stdout: &str) -> Expected<'_> {
    Expected {
        success: true,
        stdout,
        report: None,
    }
}

fn reported<'a>(success: bool, stdout: &'a str, report: &'a str) -> Expected<'a> {
    Expected {
        success,
        stdout,
        report: Some(report),
    }
}

#[test]
fn fractional_exponent() {
    for (name, expression) in [("literal", "2.0 ** 0.5"), ("computed", "2 ** (1.0 / 2)")] {
        check_backends(
            name,
            &format!("fn main() {{ let result = {expression} }}"),
            reported(false, "", "実行時エラー: ** の指数は整数値に限る"),
        );
    }
}

#[test]
fn wait_and_exponent_migration() {
    let source = r#"
use std -> *
use std::math
fn work(n: int) -> int with Async { n + 1 }
fn main() with Async {
    let work_to_run = work(1)
    put(work_to_run.await)
    spawn(work(2)).then(fn(n) { put(n) })
    put(math::sqrt(2.0))
    put(2.0 ** -3.0)
    put(2 ** 3)
}
"#;
    check_backends(
        "migration",
        source,
        success("2\n1.4142135623730951\n0.125\n8\n3\n"),
    );
}

#[test]
fn spawned_arithmetic_failure() {
    for (name, expression, message) in [
        ("division", "1 / n", "実行時エラー: 0 で除算した"),
        (
            "addition",
            "9007199254740991 + (n + 1)",
            "実行時エラー: オーバーフロー",
        ),
        (
            "multiplication",
            "9007199254740991 * (n - 10000)",
            "実行時エラー: オーバーフロー",
        ),
        ("power", "2 ** (n + 53)", "実行時エラー: オーバーフロー"),
    ] {
        let source = r#"
use std -> *
fn step(n: int) -> int with Async { n }
fn calculate(n: int) -> int { EXPRESSION }
fn bad() -> int with Async {
    let zero = step(0).await
    calculate(zero)
}
fn good() -> int with Async { step(42).await }
fn main() {
    spawn(bad()).then(fn(n) { put('失敗したタスクの then') })
    spawn(good()).then(fn(n) { put(n) })
    put('main')
}
"#;
        check_backends(
            name,
            &source.replace("EXPRESSION", expression),
            reported(true, "main\n42\n", message),
        );
    }
}

#[test]
fn failing_then() {
    let source = r#"
use std -> *
fn step() -> int with Async { 42 }
fn main() {
    let task = spawn(step())
    task.then(fn(n) { panic('then の失敗 ' + n.to_string()) })
    task.then(fn(n) { put(n / (n - n)) })
    task.then(fn(n) { put(n) })
    spawn(step()).then(fn(n) { spawn(step()).then(fn(m) { put(m + n) }) })
    put('main')
}
"#;
    let message = "実行時エラー: then の失敗 42";
    check_backends(
        "failing-then",
        source,
        reported(false, "main\n42\n84\n", message),
    );
}

#[test]
fn task_display() {
    let source = r#"
use std -> *
fn step() -> int with Async { 42 }
fn main() {
    let task = spawn(step())
    put(task)
    task.then(fn(n) { put([task]) })
}
"#;
    check_backends("task-display", source, success("<task>\n[<task>]\n"));
}

#[test]
fn nested_string_display() {
    let source = r#"
fn main() {
    let mut text = '日本語🍵'
    for c in [0, 1, 8, 9, 10, 11, 12, 13, 31, 34, 92, 127, 128, 159, 160] {
        text = text + char_from_codepoint(c)
    }
    put([[text]])
    put(char_from_codepoint(9))
}
"#;
    let quoted = r#"[["日本語🍵\u0000\u0001\b\t\n\u000b\f\r\u001f\"\\\u007f\u0080\u009f"#;
    let output = format!("{quoted}\u{a0}\"]]\n\t\n");
    check_backends("nested-string-display", source, success(&output));
}
