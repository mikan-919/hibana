use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

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

#[test]
fn compiler_services_return_data() {
    let source = "extern { fn notify(s: str) }\nextern { fn unused() }\nfn echo(s: str) -> str { notify(s); s }";
    let names = vec!["echo".into()];
    assert!(hibana::pipeline::compile(source, "wasm", &names).unwrap().starts_with(b"\0asm"));
    let metadata: serde_json::Value = serde_json::from_str(&hibana::pipeline::inspect_exports(source, &names).unwrap()).unwrap();
    assert_eq!(metadata["exports"], serde_json::json!([["echo", ["str"], "str"]]));
    assert_eq!(metadata["imports"], serde_json::json!([["notify", ["str"], "unit"]]));
    assert!(hibana::pipeline::compile(source, "unknown", &names).is_err());
    assert!(hibana::pipeline::compile(source, "wasm", &[]).is_err());
    assert!(hibana::pipeline::compile("fn main() {}", "js", &names).is_err());
}

#[test]
fn script_primitives_validate_arguments() {
    for body in ["utf8(1)", "compile(1, 'wasm', [])", "compile('fn f() {}', 1, [])", "compile('fn f() {}', 'wasm', [1])", "inspect_exports(1, [])", "inspect_exports('fn f() {}', [1])"] {
        assert!(hibana::run(&format!("fn main() {{ {body}; }}"), &mut Vec::new()).is_err(), "{body}");
    }
}

#[test]
fn script_builds_files_and_preserves_arguments() {
    let dir = temp();
    std::fs::create_dir(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.hb"), "fn answer() -> int { 42 }\n").unwrap();
    std::fs::write(dir.join("src/app.hb"), "use './lib.hb' -> *\nfn main() { put(answer()) }\n").unwrap();
    std::fs::write(dir.join("src/wasm.hb"), "use './lib.hb' -> *\nfn answer2() -> int { answer() }\n").unwrap();
    std::fs::write(dir.join("build.hb"), r#"
fn arguments() -> Vec<str> { args() }
fn main() {
    put(arguments())
    write_bytes('dist/app.mjs', compile_file('src/app.hb', 'js', []))
    write_bytes('dist/app.native', compile_file('src/app.hb', 'native', []))
    write_bytes('dist/app.wasm', compile_file('src/wasm.hb', 'wasm', ['answer2']))
    write_text('dist/glue.mjs', js_glue(inspect_file_exports('src/wasm.hb', ['answer2'])))
    write_bytes('dist/copy', read_bytes('dist/app.wasm'))
    put(read_text('src/lib.hb'))
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
