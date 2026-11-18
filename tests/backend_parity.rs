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
fn closure_assignments() {
    let source = r#"
fn main() {
    let mut n = 0
    let mut values = [0]
    let mut shared = new_ref(0)
    let original = shared
    let next = fn() {
        n = n + 1
        values = push(values, n)
        write(shared, read(shared) + 1)
        put(n)
        put(values)
        put(read(shared))
    }
    n = 10
    values = push(values, n)
    shared = new_ref(10)
    next()
    next()
    put(n)
    put(values)
    put(read(shared))
    put(read(original))
}
"#;
    check_backends(
        "closure-assignments",
        source,
        success("1\n[0, 1]\n1\n1\n[0, 1]\n2\n10\n[0, 10]\n10\n2\n"),
    );
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
fn failing_sync_main_drains_started_tasks() {
    let source = r#"
use std -> *
fn step(n: int) -> int with Async { n }
fn work() -> int with Async {
    let n = step(42).await
    put(n)
    n
}
fn fail() { panic('main の失敗 ' + 42.to_string()) }
fn main() {
    let task = spawn(work())
    task.then(fn(n) { panic('then の失敗 ' + n.to_string()) })
    task.then(fn(n) { put(n + 1) })
    fail()
    put('失敗後の main')
}
"#;
    check_backends(
        "failing-sync-main",
        source,
        reported(false, "42\n43\n", "実行時エラー: main の失敗 42"),
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
fn inferred_requirements_preserve_function_and_refinement_types() {
    let source = r#"
use std -> *
type Mapper = (int) -> String with Async
type Digit = int[d -> 0 <= d && d <= 9]
trait DigitSource<T> { fn fetch_digit(self) -> T }
trait Source<T> { fn take_value(self) -> T }

fn make_mapper() -> Mapper MAPPER_REQUIREMENT {
    let f: Mapper = take_value()
    f
}
fn mapped(n: int) -> String MAPPED_REQUIREMENTS {
    let f = make_mapper()
    f(n).await
}
fn declared_digit() -> Digit with DigitSource<Digit> { fetch_digit() }
fn get_digit() -> Digit DIGIT_REQUIREMENT { declared_digit() }
fn next_digit() -> int DIGIT_REQUIREMENT { get_digit() + 1 }
fn to_label(n: int) -> String with Async { n.to_string() }
fn main() with Async {
    put(with Source(fn() { to_label }) { mapped(42).await })
    put(with DigitSource(fn() { 8 }) { next_digit() })
}
"#;
    for explicit in [false, true] {
        let source = source
            .replace("MAPPER_REQUIREMENT", if explicit { "with Source<Mapper>" } else { "" })
            .replace("MAPPED_REQUIREMENTS", if explicit { "with Source<Mapper>, Async" } else { "" })
            .replace("DIGIT_REQUIREMENT", if explicit { "with DigitSource<Digit>" } else { "" });
        let name = if explicit { "requirements-explicit" } else { "requirements-inferred" };
        check_backends(name, &source, success("42\n9\n"));
    }
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

#[test]
fn number_display_and_json_round_trip() {
    let source = r#"
use std -> *
fn main() {
    for n in [1000000000000000.25, -1000000000000000.25, 1000000000000000.75, -0.0, 100000000000000000000000.0] {
        put(n)
        put(n.to_string())
        let json = stringify(JNum(n))
        put(json)
        match parse_json(json) {
            Some(JNum(parsed)) -> {
                put(parsed == n)
                put([parsed])
            }
            _ -> panic('JNum が必要')
        }
    }
}
"#;
    check_backends(
        "number_display",
        source,
        success(concat!(
            "1000000000000000.2\n1000000000000000.2\n1000000000000000.2\ntrue\n[1000000000000000.2]\n",
            "-1000000000000000.2\n-1000000000000000.2\n-1000000000000000.2\ntrue\n[-1000000000000000.2]\n",
            "1000000000000000.8\n1000000000000000.8\n1000000000000000.8\ntrue\n[1000000000000000.8]\n",
            "-0\n-0\n-0.0\ntrue\n[-0]\n",
            "100000000000000000000000\n100000000000000000000000\n100000000000000000000000.0\ntrue\n[100000000000000000000000]\n",
        )),
    );
}

#[test]
fn equality_by_identity() {
    let source = r#"
use std -> *
struct Handles { task: Task<int>, reference: Ref<int> }
fn step() -> int with Async { 42 }
fn id(n: int) -> int { n }
fn main() {
    let t = spawn(step())
    let alias = t
    let other = spawn(step())
    let r = new_ref(1)
    let same = r
    let different = new_ref(1)
    put([t == alias, t == other, t != other])
    put([r == same, r == different, r != different])
    write(same, 2)
    write(different, 2)
    let h = Handles { task: t, reference: r }
    let shared = Handles { task: alias, reference: same }
    let separate = Handles { task: t, reference: different }
    put([r == same, [t] == [alias], [t] == [other], [h] == [shared], h != separate])
    let callable = new_ref(id)
    put(callable == callable)
    other.then(fn(n) { put([t == alias, t == other, h == shared]) })
}
"#;
    check_backends(
        "equality-by-identity",
        source,
        success("[true, false, true]\n[true, false, true]\n[true, true, false, true, true]\ntrue\n[true, false, true]\n"),
    );
}

#[test]
fn equality_trait_dispatch() {
    check_backends(
        "existing-eq",
        include_str!("programs/types/eq.hb"),
        success("true\ntrue\n"),
    );
    let source = r#"
use std -> *
type Callback = (int) -> int
struct Job { id: int, run: Callback }
impl Eq for Job {
    fn eq(self, other: Job) -> bool { self.id == other.id }
}
fn same<A: Eq>(a: A, b: A) -> bool { a == b }
fn same_options<A: Eq>(a: Option<A>, b: Option<A>) -> bool { a == b }
enum Chain<A> { End, Link(rest: Chain<A>, value: A) }
fn main() {
    let a = Job { id: 1, run: fn(n) { n } }
    let b = Job { id: 1, run: fn(n) { n + 1 } }
    let c = Job { id: 2, run: a.run }
    put([a == b, a != c, same(a, b), same_options(Some(a), Some(b)), [a] == [b]])
    let end: Chain<int> = End
    let xs = Link(end, 1)
    put([xs == Link(end, 1), xs == Link(end, 2)])
}
"#;
    check_backends(
        "equality-trait-dispatch",
        source,
        success("[true, true, true, true, true]\n[true, false]\n"),
    );
}

#[test]
fn equality_type_errors() {
    for (declarations, body, message) in [
        ("", "let a = id\nlet b = id\nput(a == b)", "は比較できない"),
        ("", "put(step() != step())", "Async<int> は比較できない"),
        ("", "put([[id]] == [[id]])", "は比較できない"),
        ("struct Box<A> { value: A }", "put(Box { value: step() } == Box { value: step() })", "Async<int> は比較できない"),
        (
            "enum Chain<A> { End, Link(rest: Chain<A>, value: A) }",
            "let end: Chain<(int) -> int> = End\nput(Link(end, id) != Link(end, id))",
            "は比較できない",
        ),
        ("fn same<A>(a: A, b: A) -> bool { a != b }", "put(same(id, id))", "<A: Eq>"),
        ("fn same<A>(a: Vec<A>, b: Vec<A>) -> bool { a == b }", "put(same([step()], [step()]))", "組み込みの比較"),
        ("fn same<A: Eq>(a: A, b: A) -> bool { a == b }", "put(same(id, id))", "は比較できない"),
        ("fn same<A: Eq>(a: A, b: A) -> bool { a == b }", "put(same(step(), step()))", "Async<int> は比較できない"),
        (
            "type Callback = (int) -> int
             impl Eq for Callback { fn eq(self, other: Callback) -> bool { true } }
             fn same<A: Eq>(a: A, b: A) -> bool { a == b }",
            "put(same(id, id))",
            "は比較できない",
        ),
        (
            "struct Job { run: (int) -> int }
             impl Eq for Job { fn eq(self, other: Job) -> bool { true } }
             struct Wrapped<A> { value: A }
             fn same<A: Eq>(a: Wrapped<A>, b: Wrapped<A>) -> bool { a == b }",
            "put(same(Wrapped { value: Job { run: id } }, Wrapped { value: Job { run: id } }))",
            "組み込みの比較",
        ),
    ] {
        let source = format!(
            "use std -> *\nfn id(n: int) -> int {{ n }}\nfn step() -> int with Async {{ 42 }}\n{declarations}\nfn main() {{ {body} }}"
        );
        let errors = [
            hibana::run(&source, &mut Vec::new()).err(),
            hibana::to_js(&source).err(),
            hibana::to_web(&source, &["main".into()]).err(),
            hibana::to_native(&source).err(),
        ];
        let messages = errors.map(|error| error.expect(&source).to_string());
        assert!(messages[0].contains(message), "{source}\n{}", messages[0]);
        assert!(messages.iter().all(|m| *m == messages[0]), "{messages:?}");
    }
}
