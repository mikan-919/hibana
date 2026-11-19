use std::process::Command;

#[test]
fn removed_project_commands_are_rejected() {
    for command in [
        "new", "add", "fetch", "install", "update", "remove", "dlx", "build", "dev",
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_hibana"))
            .arg(command)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{command}: {result:?}");
        assert!(String::from_utf8_lossy(&result.stderr).contains("不明なサブコマンド"));
    }
}

#[test]
fn tests_require_explicit_files_and_missing_imports_report_the_path() {
    let result = Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg("test")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("ファイル"));
    let error = hibana::pipeline::load("use missing_library\nfn main() {}", std::path::Path::new(".")).unwrap_err();
    assert!(error
        .to_string()
        .contains("ライブラリ missing_library を読み込めない"));
    assert!(error.to_string().contains("missing_library.hb"));
}

#[test]
fn script_arguments_are_rejected() {
    for extra in [vec!["--", "value"], vec!["value"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_hibana"))
            .arg("examples/first.hb")
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&result.stderr).contains("使い方"));
    }
}

#[test]
fn removed_build_names_are_ordinary_functions() {
    for name in [
        "compile", "inspect_exports", "utf8", "args", "read_text", "read_bytes",
        "write_text", "inspect_file_exports", "js_glue",
    ] {
        let source = format!("fn main() {{ {name}() }}");
        assert!(hibana::run(&source, &mut Vec::new()).is_err(), "{name}");
        let source = format!("fn {name}() -> int {{ 42 }} fn main() {{ put({name}()) }}");
        let mut output = Vec::new();
        hibana::run(&source, &mut output).unwrap();
        assert_eq!(output, b"42\n", "{name}");
    }
}

// ビルドスクリプトは compile_file と write_bytes で JS・WASM・ネイティブを書き出す。
// use はコンパイルするファイルのディレクトリから探し、パスは作業ディレクトリから数える
#[test]
fn build_scripts_write_compiled_outputs() {
    let dir = std::env::temp_dir().join(format!("hibana-build-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/greet.hb"), "pub fn greet() -> String with {} { 'hello' }\n").unwrap();
    std::fs::write(dir.join("src/app.hb"), "use greet\nfn main() {\n    put(greet.greet())\n}\n").unwrap();
    std::fs::write(
        dir.join("build.hb"),
        "fn main() {\n    for t in ['js', 'wasm', 'native'] {\n        write_bytes('dist/app.' + t, compile_file('src/app.hb', t))\n    }\n}\n",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_hibana")).arg("build.hb").current_dir(&dir).output().unwrap();
    assert!(result.status.success(), "{result:?}");
    let js = Command::new("node").arg(dir.join("dist/app.js")).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&js.stdout), "hello\n");
    assert!(std::fs::read(dir.join("dist/app.wasm")).unwrap().starts_with(b"\0asm"));
    assert!(std::fs::read(dir.join("dist/app.native")).unwrap().starts_with(b"\x7fELF"));
    let result = Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg("../build.hb")
        .current_dir(dir.join("src"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&result.stderr).contains("src/app.hb"), "{result:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hibana_build_scripts_generate_web_library_and_typescript() {
    let dir = std::env::temp_dir().join(format!("hibana-build-web-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/rules.hb"), "pub type Positive = int[x -> x > 0]\n").unwrap();
    std::fs::write(dir.join("src/app.hb"), r#"
use rules
pub fn validate(n: int) -> Result<rules.Positive, String> with {} {
    refine<rules.Positive>(n)
}
"#).unwrap();
    std::fs::write(dir.join("build.hb"), r#"
fn main() {
    let exports = ['validate']
    write_bytes('dist/app.wasm', compile_web_file('src/app.hb', 'wasm', exports))
    write_bytes('dist/app.mjs', compile_web_file('src/app.hb', 'js', exports))
    write_bytes('dist/app.d.mts', compile_web_file('src/app.hb', 'ts', exports))
}
"#).unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg("build.hb").current_dir(&dir).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let declarations = std::fs::read_to_string(dir.join("dist/app.d.mts")).unwrap();
    assert!(declarations.contains(r#""validate": (arg0: number) => { ok: true; value: number } | { ok: false; error: string };"#));
    let js = Command::new("node").args(["--input-type=module", "-e", r#"
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { instantiate } from './dist/app.mjs';
const api = await instantiate(readFileSync('dist/app.wasm'));
assert.deepEqual(Object.keys(api), ['validate']);
assert.deepEqual(api.validate(7), { ok: true, value: 7 });
assert.deepEqual(api.validate(0), { ok: false, error: '細分化型の条件を満たさない' });
"#]).current_dir(&dir).output().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(js.status.success(), "{}", String::from_utf8_lossy(&js.stderr));
}

#[test]
fn script_functions_are_rejected_by_compiled_backends() {
    for src in [
        "fn main() { write_bytes('x', [1]) }",
        "fn main() { put(len(compile_web_file('src/app.hb', 'ts', ['validate']))) }",
    ] {
        for e in [hibana::to_js(src).unwrap_err(), hibana::to_wasm(src).unwrap_err(), hibana::to_native(src).unwrap_err()] {
            assert!(e.to_string().contains("インタプリタでの実行"), "{e}");
        }
    }
    assert!(hibana::to_js("fn f() -> Vec<int> { compile_file('a.hb', 'js') }\nfn main() {}").is_err());
}
