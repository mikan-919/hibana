use hibana::pipeline::build_artifacts;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const EMIT: &str = r#"
fn emit_asset(path: str, text: str) {
    artifact(path, utf8(text))
}
macro asset ("asset" path:String "," text:String) -> Decl {
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

fn cli(source: &str, dir: &std::path::Path) -> std::process::Output {
    let input = dir.join("build.hb");
    std::fs::write(&input, source).unwrap();
    Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg("build")
        .arg(input)
        .arg("--out-dir")
        .arg(dir.join("dist"))
        .output()
        .unwrap()
}

#[test]
fn macro_can_leave_runtime_empty() {
    let src = format!("{EMIT}\nasset \"nested/hello.txt\", \"こんにちは\"");
    let artifacts = build_artifacts(&src).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].name, "nested/hello.txt");
    assert_eq!(artifacts[0].contents, "こんにちは".as_bytes());
    let (program, _) = hibana::build::capture(|| hibana::parser::parse(&src));
    assert!(program.unwrap().fns.is_empty());
    let dir = temp();
    let result = cli(&src, &dir);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("dist/nested/hello.txt")).unwrap(),
        "こんにちは"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compile_time_helpers_preserve_shared_runtime_functions() {
    let src = r#"
macro asset ("asset") -> Decl {
    emit_asset()
    quote {}
}
fn emit_asset() {
    artifact("shared.txt", utf8(shared()))
}
fn shared() -> str { "shared" }
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
fn all_checks_and_generation_finish_before_writes() {
    for tail in [
        "fn broken() -> int { true }",
        "macro bad (\"bad\") -> Decl { artifact(\"bad.wasm\", compile(\"fn f() {}\", \"wasm\", [\"missing\"])); quote {} }\nbad",
        "macro bad (\"bad\") -> Decl { artifact(\"other.txt\", utf8(\"x\")); panic(\"failed\"); quote {} }\nbad",
    ] {
        let dir = temp();
        let result = cli(&format!("{EMIT}\nasset \"ok.txt\", \"ok\"\n{tail}"), &dir);
        assert!(!result.status.success());
        assert!(!dir.join("dist").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    assert!(build_artifacts(&format!("{EMIT}\nasset \"good.txt\", \"good\"")).is_ok());
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
                "{EMIT}\nasset {}, \"x\"",
                serde_json::to_string(path).unwrap()
            ))
            .is_err(),
            "{path}"
        );
    }
    for second in ["a", "a/b"] {
        assert!(
            build_artifacts(&format!(
                "{EMIT}\nasset \"a\", \"x\"\nasset \"{second}\", \"y\""
            ))
            .is_err()
        );
    }
}

#[test]
fn imported_library_controls_outputs() {
    let dir = temp();
    std::fs::write(dir.join("assets.hb"), EMIT).unwrap();
    let result = cli(
        "use \"./assets.hb\"\nasset \"library.txt\", \"from library\"",
        &dir,
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
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
web_build "app.wasm", "app.mjs" source "extern { fn decorate(s: str) -> str } fn echo(s: str) -> str { decorate(s) }" exports (echo)
"#;
    let dir = temp();
    let result = cli(src, &dir);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
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

#[cfg(unix)]
#[test]
fn output_symlinks_and_filesystem_collisions_prevent_all_writes() {
    for kind in ["root", "parent", "file", "directory", "blocked"] {
        let dir = temp();
        let out = dir.join("dist");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        if kind == "root" {
            std::os::unix::fs::symlink(&elsewhere, &out).unwrap();
        } else {
            std::fs::create_dir_all(&out).unwrap();
            match kind {
                "parent" => std::os::unix::fs::symlink(&elsewhere, out.join("nested")).unwrap(),
                "file" => {
                    std::fs::create_dir(out.join("nested")).unwrap();
                    std::os::unix::fs::symlink(elsewhere.join("target"), out.join("nested/x"))
                        .unwrap();
                }
                "directory" => std::fs::create_dir_all(out.join("nested/x")).unwrap(),
                "blocked" => std::fs::write(out.join("nested"), "file").unwrap(),
                _ => unreachable!(),
            }
        }
        let result = cli(
            &format!("{EMIT}\nasset \"first.txt\", \"x\"\nasset \"nested/x\", \"y\""),
            &dir,
        );
        assert!(!result.status.success(), "{kind}");
        assert!(!out.join("first.txt").exists());
        assert!(!elsewhere.join("first.txt").exists());
        assert!(!elsewhere.join("x").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
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
    assert!(build_artifacts("fn f() { artifact(\"x\", utf8(\"y\")) } fn main() { f() }").is_err());
    assert!(hibana::to_js(&format!("{EMIT}\nasset \"x\", \"y\"\nfn main() {{}}")).is_err());
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
    assert!(hibana::pipeline::compile(source, "wasm", &[]).is_err());
    assert!(hibana::pipeline::compile("fn main() {}", "js", &names).is_err());
    for target in ["js", "c"] {
        assert!(
            !hibana::pipeline::compile("fn main() {}", target, &[])
                .unwrap()
                .is_empty()
        );
    }
    let nested = r#"macro output ("output") -> Decl { artifact("nested", [0]); quote {} } output"#;
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
    artifact(path, utf8("<h1>" + heading + "</h1>"))
    artifact("data.bin", [0, 127, 128, 255])
}
macro site ("site" heading:String) -> Decl {
    custom("index.html", str_of(heading))
    quote {}
}
site "ビルド"
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
            "macro asset (\"asset\") -> Decl {{ artifact(\"x\", [{byte}]); quote {{}} }} asset"
        );
        assert!(build_artifacts(&source).is_err());
    }
}

#[test]
fn compiler_results_can_be_published_by_library_helpers() {
    let source = r#"
fn generate() {
    let bytes = compile("fn main() { print(42) }", "js", [])
    artifact("program.mjs", bytes)
}
macro make ("make") -> Decl { generate(); quote {} }
make
"#;
    let dir = temp();
    let result = cli(source, &dir);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
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
        "artifact(\"x\", [true])",
        "utf8(1)",
        "compile(1, \"wasm\", [])",
        "compile(\"fn f() {}\", 1, [])",
        "compile(\"fn f() {}\", \"wasm\", [1])",
        "inspect_exports(1, [])",
        "inspect_exports(\"fn f() {}\", [1])",
    ] {
        let source = format!("macro asset (\"asset\") -> Decl {{ {body}; quote {{}} }} asset");
        assert!(build_artifacts(&source).is_err(), "{body}");
    }
}
