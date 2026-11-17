use std::process::Command;

#[test]
fn async_annotations_survive_cloning_and_numeric_elaboration() {
    let source = r#"
extern '' { fn delay(n: int) -> int with Async }
fn apply<E: Effect>(f: () -> int with E) -> int with E { f() }
fn main() with Async {
    let result: num = apply(fn() { delay(7).await })
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
globalThis.pause = async () => 0;",
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
    for operand in ["(1 - 1)", "pause().await"] {
        let source = format!(
            "extern '' {{ fn pause() -> int with Async }}\n\
fn main() with Async {{\n    put(1 / {operand})\n}}\n"
        );
        let output = javascript_error(&source, "globalThis.pause = async () => 0;");
        assert!(!output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "実行時エラー: 0 で除算した（3 行 9 列）\n"
        );
    }
}
