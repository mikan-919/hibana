use std::fs;
use std::process::Command;

struct Expected<'a> {
    success: bool,
    stdout: &'a str,
    report: Option<&'a str>,
}

fn check_backends(name: &str, source: &str, expected: [Expected<'_>; 4]) {
    let dir = std::env::temp_dir().join(format!("hibana-parity-{name}-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();

    let hb = dir.join("main.hb");
    fs::write(&hb, source).unwrap();
    let interpreted = Command::new(env!("CARGO_BIN_EXE_hibana")).arg(hb).output().unwrap();

    let javascript = dir.join("main.cjs");
    let js = hibana::to_js(source).unwrap();
    fs::write(&javascript, format!("process.on('uncaughtException', e => {{ console.error(e.message); process.exitCode = 1; }});\n{js}")).unwrap();
    let js = Command::new("node").arg(javascript).output().unwrap();

    let web = hibana::to_web(source, &["main".into()]).unwrap();
    fs::write(dir.join("main.wasm"), web.wasm).unwrap();
    fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let runner = dir.join("run.mjs");
    fs::write(&runner, r#"
import {readFileSync} from 'node:fs';
import {instantiate} from './glue.mjs';
const api = await instantiate(readFileSync(new URL('./main.wasm', import.meta.url)));
try { api.main(); } catch (e) { console.error(e.message); process.exitCode = 1; }
"#).unwrap();
    let wasm = Command::new("node").arg(runner).output().unwrap();
    let mut outputs = vec![("インタプリタ", interpreted), ("JavaScript", js), ("WASM", wasm)];

    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let native = dir.join("main");
        fs::write(&native, hibana::to_native(source).unwrap()).unwrap();
        fs::set_permissions(&native, fs::Permissions::from_mode(0o755)).unwrap();
        outputs.push(("ネイティブ", Command::new(native).output().unwrap()));
    }

    for ((backend, output), want) in outputs.into_iter().zip(expected) {
        let stderr = std::str::from_utf8(&output.stderr).unwrap();
        assert_eq!(output.status.success(), want.success, "{name} / {backend}: {stderr}");
        assert_eq!(std::str::from_utf8(&output.stdout).unwrap(), want.stdout, "{name} / {backend}");
        if let Some(report) = want.report {
            let mut lines = stderr.lines().filter(|line| !line.trim().is_empty());
            assert!(lines.next().is_some_and(|line| line.contains(report)), "{name} / {backend}: {stderr}");
            assert_eq!(lines.filter(|line| line.contains("実行時エラー:")).count(), 0, "{name} / {backend}: {stderr}");
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
    Expected { success: true, stdout, report: None }
}

fn reported<'a>(success: bool, stdout: &'a str, report: &'a str) -> Expected<'a> {
    Expected { success, stdout, report: Some(report) }
}

#[test]
fn fractional_exponent() {
    check_backends("fractional-exponent", "fn main() { put(2.0 ** 0.5) }", [
        success("1.4142135623730951\n"),
        success("1.4142135623730951\n"),
        reported(false, "", "unreachable"),
        reported(false, "", "実行時エラー: number の小数の指数は未対応"),
    ]);
}

#[test]
fn spawned_division_by_zero() {
    let source = r#"
use std -> *
fn step(n: int) -> int with Async { n }
fn divide(n: int) -> int { 1 / n }
fn bad() -> int with Async {
    let zero = step(0).await
    divide(zero)
}
fn good() -> int with Async { step(42).await }
fn main() {
    spawn(bad()).then(fn(n) { put('失敗したタスクの then') })
    spawn(good()).then(fn(n) { put(n) })
    put('main')
}
"#;
    let message = "実行時エラー: 0 で除算した";
    check_backends("spawned-division", source, [
        reported(true, "main\n42\n", message),
        reported(true, "main\n42\n", message),
        reported(false, "main\n", "unreachable"),
        reported(false, "main\n", message),
    ]);
}

#[test]
fn failing_then() {
    let source = r#"
use std -> *
fn step() -> int with Async { 42 }
fn main() {
    let task = spawn(step())
    task.then(fn(n) { panic('then の失敗') })
    task.then(fn(n) { put(n) })
    put('main')
}
"#;
    let message = "実行時エラー: then の失敗";
    check_backends("failing-then", source, [
        reported(false, "main\n42\n", message),
        reported(false, "main\n42\n", "then の失敗"),
        reported(false, "main\n", "unreachable"),
        reported(false, "main\n", message),
    ]);
}

#[test]
fn task_display() {
    let source = "fn step() -> int with Async { 42 } fn main() { put(spawn(step())) }";
    check_backends("task-display", source, [
        success("<task>\n"),
        reported(false, "", "実行時エラー:"),
        success("(0, (), <ref>)\n"),
        success("(0, (), <ref>)\n"),
    ]);
}

#[test]
fn nested_string_display() {
    let source = "fn main() { put([[char_from_codepoint(8) + char_from_codepoint(0)]]) }";
    check_backends("nested-string-display", source, [
        success("[[\"\\u{8}\\0\"]]\n"),
        success("[[\"\\b\\u0000\"]]\n"),
        success("[[\"\x08\\0\"]]\n"),
        success("[[\"\x08\\0\"]]\n"),
    ]);
}
