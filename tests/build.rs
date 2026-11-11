use hibana::pipeline::build_artifacts;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const EMIT: &str = r#"
macro asset ("asset" path:String "," text:String) -> Decl {
    build_emit(str_of(path), str_of(text))
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
fn all_checks_and_generation_finish_before_writes() {
    for tail in [
        "fn broken() -> int { true }",
        "macro bad (\"bad\") -> Decl { build_wasm(\"bad.wasm\", [\"missing\"]); quote {} }\nbad",
        "macro bad (\"bad\") -> Decl { build_emit(\"other.txt\", \"x\"); panic(\"failed\"); quote {} }\nbad",
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
macro bundle ("bundle") -> Decl {
    build_wasm("app.wasm", ["echo"])
    build_glue("app.mjs", ["echo"])
    quote {}
}
fn echo(s: str) -> str { s }
bundle
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
const api = await instantiate(await readFile(new URL('./dist/app.wasm', import.meta.url)));
if (api.echo('ビルド🌸') !== 'ビルド🌸') throw Error('result');
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
    assert!(build_artifacts("fn f() { build_emit(\"x\", \"y\") }").is_err());
    assert!(hibana::to_js(&format!("{EMIT}\nasset \"x\", \"y\"\nfn main() {{}}")).is_err());
}
