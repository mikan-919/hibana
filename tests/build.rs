use hibana::pipeline::build_artifacts;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const EMIT: &str = r#"
fn emit_asset(path: str, text: str) {
    artifact(path, utf8(text))
}
macro asset ('asset' path:String ',' text:String) -> Decl {
    emit_asset(str_of(path), str_of(text))
    quote {}
}
"#;

fn temp() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hibana-build-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// Only the test harness writes artifacts; the public API returns their data.
fn write_artifacts(source: &str, dir: &std::path::Path) {
    let artifacts = build_artifacts(source).unwrap();
    for artifact in artifacts {
        let path = dir.join("dist").join(artifact.name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, artifact.contents).unwrap();
    }
}

#[test]
fn macro_can_leave_runtime_empty() {
    let src = format!("{EMIT}\nasset 'nested/hello.txt', 'こんにちは'");
    let artifacts = build_artifacts(&src).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].name, "nested/hello.txt");
    assert_eq!(artifacts[0].contents, "こんにちは".as_bytes());
    let (program, _) = hibana::build::capture(|| hibana::parser::parse(&src));
    assert!(program.unwrap().fns.is_empty());
}

#[test]
fn compile_time_helpers_preserve_shared_runtime_functions() {
    let src = r#"
macro asset ('asset') -> Decl {
    emit_asset()
    quote {}
}
fn emit_asset() {
    artifact('shared.txt', utf8(shared()))
}
fn shared() -> str { 'shared' }
fn main() { print(shared()) }
asset
"#;
    let artifacts = build_artifacts(src).unwrap();
    assert_eq!(artifacts[0].contents, b"shared");
    let (program, _) = hibana::build::capture(|| hibana::parser::parse(src));
    let program = program.unwrap();
    let names: Vec<_> = program.fns.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"main"));
    assert!(names.contains(&"shared"));
    assert!(!names.contains(&"emit_asset"));
    hibana::pipeline::check(program, hibana::pipeline::Kind::Executable).unwrap();
}

#[test]
fn all_checks_and_generation_must_succeed_before_returning_artifacts() {
    for tail in [
        "fn broken() -> int { true }",
        "macro bad ('bad') -> Decl { artifact('bad.wasm', compile('fn f() {}', 'wasm', ['missing'])); quote {} }\nbad",
        "macro bad ('bad') -> Decl { artifact('other.txt', utf8('x')); panic('failed'); quote {} }\nbad",
    ] {
        assert!(build_artifacts(&format!("{EMIT}\nasset 'ok.txt', 'ok'\n{tail}")).is_err());
    }
    assert!(build_artifacts(&format!("{EMIT}\nasset 'good.txt', 'good'")).is_ok());
}

#[test]
fn rejects_unsafe_and_colliding_artifact_paths() {
    for path in [
        "",
        "../outside",
        "/absolute",
        "a/../b",
        "a//b",
        "./a",
        "C:/a",
        "a\\b",
    ] {
        assert!(
            build_artifacts(&format!(
                "{EMIT}\nasset '{}', 'x'",
                path.replace('\\', "\\\\")
            ))
            .is_err(),
            "{path}"
        );
    }
    for second in ["a", "a/b"] {
        assert!(
            build_artifacts(&format!(
                "{EMIT}\nasset 'a', 'x'\nasset '{second}', 'y'"
            ))
            .is_err()
        );
    }
}

#[test]
fn imported_library_controls_outputs() {
    let dir = temp();
    std::fs::write(dir.join("assets.hb"), EMIT).unwrap();
    write_artifacts(
        &format!(
            "use '{}'\nasset 'library.txt', 'from library'",
            dir.join("assets.hb").to_str().unwrap()
        ),
        &dir,
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("dist/library.txt")).unwrap(),
        "from library"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn library_macro_generates_wasm_and_glue() {
    let src = r#"
use build_web
web_build 'app.wasm', 'app.mjs' source 'extern { fn decorate(s: str) -> str } fn echo(s: str) -> str { decorate(s) }' exports (echo)
"#;
    let dir = temp();
    write_artifacts(src, &dir);
    let script = dir.join("check.mjs");
    std::fs::write(
        &script,
        r#"
import {readFile} from 'node:fs/promises';
import {instantiate} from './dist/app.mjs';
const api = await instantiate(await readFile(new URL('./dist/app.wasm', import.meta.url)), {
  hibana_host: { decorate: text => text + '!' }
});
if (api.echo('ビルド🌸') !== 'ビルド🌸!') throw Error('result');
"#,
    )
    .unwrap();
    let result = Command::new("node").arg(script).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn documented_build_library_fixture() {
    let src = include_str!("fixtures/build_artifacts.hb");
    let artifacts = build_artifacts(src).unwrap();
    let expanded = hibana::parser::with_uses(src).unwrap();
    let (program, _) = hibana::build::capture(|| hibana::parser::parse(&expanded));
    assert!(program.unwrap().fns.is_empty());
    assert_eq!(artifacts.len(), 3);
    let get = |name: &str| &artifacts.iter().find(|a| a.name == name).unwrap().contents;
    assert!(get("app.wasm").starts_with(b"\0asm"));
    assert!(
        std::str::from_utf8(get("app.mjs"))
            .unwrap()
            .contains("export async function instantiate")
    );
    let html = std::str::from_utf8(get("index.html")).unwrap();
    assert!(html.contains("./app.mjs"));
    assert!(html.contains("./app.wasm"));
}

#[test]
fn build_builtins_are_not_runtime_functions() {
    assert!(build_artifacts("fn f() { artifact('x', utf8('y')) } fn main() { f() }").is_err());
    assert!(hibana::to_js(&format!("{EMIT}\nasset 'x', 'y'\nfn main() {{}}")).is_err());
}

#[test]
fn compiler_services_return_data_without_registering_artifacts() {
    let source = r#"
extern { fn notify(s: str) }
extern { fn unused() }
fn echo(s: str) -> str { notify(s); s }
"#;
    let names = vec!["echo".into()];
    let (_, requests) = hibana::build::capture(|| {
        let bytes = hibana::pipeline::compile(source, "wasm", &names).unwrap();
        assert!(bytes.starts_with(b"\0asm"));
        let metadata: serde_json::Value =
            serde_json::from_str(&hibana::pipeline::inspect_exports(source, &names).unwrap())
                .unwrap();
        assert_eq!(
            metadata["exports"],
            serde_json::json!([["echo", ["str"], "str"]])
        );
        assert_eq!(
            metadata["imports"],
            serde_json::json!([["notify", ["str"], "unit"]])
        );
    });
    assert!(requests.is_empty());
    assert!(hibana::pipeline::compile(source, "unknown", &names).is_err());
    assert!(hibana::pipeline::compile("fn main() {}", "c", &[]).is_err());
    assert!(hibana::pipeline::compile(source, "wasm", &[]).is_err());
    assert!(hibana::pipeline::compile("fn main() {}", "js", &names).is_err());
    for target in ["js"] {
        assert!(
            !hibana::pipeline::compile("fn main() {}", target, &[])
                .unwrap()
                .is_empty()
        );
    }
    let nested = r#"macro output ('output') -> Decl { artifact('nested', [0]); quote {} } output"#;
    let (_, requests) = hibana::build::capture(|| {
        assert!(hibana::pipeline::compile(nested, "wasm", &names).is_err());
        assert!(hibana::pipeline::inspect_exports(nested, &names).is_err());
    });
    assert!(requests.is_empty());
}

#[test]
fn library_functions_generate_custom_text_and_binary() {
    let source = r#"
fn custom(path: str, heading: str) {
    artifact(path, utf8('<h1>' + heading + '</h1>'))
    artifact('data.bin', [0, 127, 128, 255])
}
macro site ('site' heading:String) -> Decl {
    custom('index.html', str_of(heading))
    quote {}
}
site 'ビルド'
"#;
    let artifacts = build_artifacts(source).unwrap();
    assert_eq!(artifacts.len(), 2);
    assert_eq!(
        artifacts
            .iter()
            .find(|a| a.name == "index.html")
            .unwrap()
            .contents,
        "<h1>ビルド</h1>".as_bytes()
    );
    assert_eq!(
        artifacts
            .iter()
            .find(|a| a.name == "data.bin")
            .unwrap()
            .contents,
        vec![0, 127, 128, 255]
    );
    let (program, _) = hibana::build::capture(|| hibana::parser::parse(source));
    assert!(program.unwrap().fns.is_empty());
    for byte in [-1, 256] {
        let source = format!(
            "macro asset ('asset') -> Decl {{ artifact('x', [{byte}]); quote {{}} }} asset"
        );
        assert!(build_artifacts(&source).is_err());
    }
}

#[test]
fn compiler_results_can_be_published_by_library_helpers() {
    let source = r#"
fn generate() {
    let bytes = compile('fn main() { print(42) }', 'js', [])
    artifact('program.mjs', bytes)
}
macro make ('make') -> Decl { generate(); quote {} }
make
"#;
    let dir = temp();
    write_artifacts(source, &dir);
    let result = Command::new("node")
        .arg(dir.join("dist/program.mjs"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "42");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn host_primitives_validate_arguments() {
    for body in [
        "artifact(1, [])",
        "artifact('x', [true])",
        "utf8(1)",
        "compile(1, 'wasm', [])",
        "compile('fn f() {}', 1, [])",
        "compile('fn f() {}', 'wasm', [1])",
        "inspect_exports(1, [])",
        "inspect_exports('fn f() {}', [1])",
    ] {
        let source = format!("macro asset ('asset') -> Decl {{ {body}; quote {{}} }} asset");
        assert!(build_artifacts(&source).is_err(), "{body}");
    }
}

#[test]
fn script_builds_files_and_preserves_arguments() {
    let dir = temp();
    std::fs::create_dir(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.hb"), "fn answer() -> int { 42 }\n").unwrap();
    std::fs::write(dir.join("src/app.hb"), "use './lib.hb' -> *\nfn main() { print(answer()) }\n").unwrap();
    std::fs::write(dir.join("src/wasm.hb"), "use './lib.hb' -> *\nfn answer2() -> int { answer() }\n").unwrap();
    std::fs::write(dir.join("build.hb"), r#"
fn arguments() -> Vec<str> { args() }
fn main() {
    print(arguments())
    write_bytes('dist/app.mjs', compile_file('src/app.hb', 'js', []))
    write_bytes('dist/app.native', compile_file('src/app.hb', 'native', []))
    write_bytes('dist/app.wasm', compile_file('src/wasm.hb', 'wasm', ['answer2']))
    write_text('dist/glue.mjs', js_glue(inspect_file_exports('src/wasm.hb', ['answer2'])))
    write_bytes('dist/copy', read_bytes('dist/app.wasm'))
    print(read_text('src/lib.hb'))
}
"#).unwrap();
    let run = Command::new(env!("CARGO_BIN_EXE_hibana")).current_dir(&dir).args(["build.hb", "--", "a", "b"]).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8_lossy(&run.stdout).starts_with("[\"a\", \"b\"]"));
    let js = Command::new("node").arg(dir.join("dist/app.mjs")).output().unwrap();
    assert_eq!(js.stdout, b"42\n");
    std::fs::write(dir.join("dist/test.mjs"), "import { instantiate } from './glue.mjs'; import { readFileSync } from 'node:fs'; console.log((await instantiate(readFileSync(new URL('./app.wasm', import.meta.url)))).answer2().toString());").unwrap();
    let wasm = Command::new("node").arg(dir.join("dist/test.mjs")).output().unwrap();
    assert!(wasm.status.success(), "{}", String::from_utf8_lossy(&wasm.stderr));
    assert_eq!(wasm.stdout, b"42\n");
    assert_eq!(std::fs::read(dir.join("dist/app.wasm")).unwrap(), std::fs::read(dir.join("dist/copy")).unwrap());
    assert!(std::fs::read(dir.join("dist/app.native")).unwrap().starts_with(b"\x7fELF"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compiled_targets_reject_script_functions() {
    for call in ["args()", "read_text('x')", "write_bytes('x', [1])", "compile_file('x', 'js', [])", "utf8('x')"] {
        let src = format!("fn helper() {{ {call}; }}\nfn main() {{ helper() }}\n");
        for result in [hibana::to_js(&src).map(|_| ()), hibana::to_wasm(&src).map(|_| ()), hibana::to_native(&src).map(|_| ())] {
            let err = result.unwrap_err().to_string();
            assert!(err.contains("スクリプト実行"), "{call}: {err}");
        }
    }
    let source = "fn exported() -> str { read_text('x') }";
    let err = hibana::to_wasm_library(source, &["exported".into()]).unwrap_err().to_string();
    assert!(err.contains("スクリプト実行"), "{err}");
}

#[test]
fn script_rejects_invalid_bytes_without_writing() {
    let dir = temp();
    let path = dir.join("bad.hb");
    std::fs::write(&path, "fn main() { write_bytes('invalid', [256]) }\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hibana")).current_dir(&dir).arg(path).output().unwrap();
    assert!(!out.status.success());
    assert!(!dir.join("invalid").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
