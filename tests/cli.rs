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

// ビルドスクリプトは std::build と std::fs で JS・WASM・ネイティブを書き出す。
// use はコンパイルするファイルのディレクトリから、スクリプトの経路はスクリプトのディレクトリから探す
#[test]
fn build_scripts_write_compiled_outputs() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("hibana-build-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/greet.hb"), "pub fn greet() -> String with {} { 'hello' }\n").unwrap();
    std::fs::write(dir.join("src/app.hb"), "use greet\nfn main() {\n    put(greet::greet())\n}\n").unwrap();
    std::fs::write(
        dir.join("build.hb"),
        "use std::build\nuse std::fs\n\nfn main() {\n    for t in ['js', 'wasm'] {\n        fs::write_bytes('dist/app.' + t, build::compile('src/app.hb', t))\n    }\n    build::write_executable('dist/app', build::compile('src/app.hb', 'native'))\n}\n",
    )
    .unwrap();
    // 作業ディレクトリによらず、同じ場所に書き出す
    for (cwd, script) in [(dir.clone(), "build.hb"), (dir.join("src"), "../build.hb")] {
        let _ = std::fs::remove_dir_all(dir.join("dist"));
        let result = Command::new(env!("CARGO_BIN_EXE_hibana")).arg(script).current_dir(&cwd).output().unwrap();
        assert!(result.status.success(), "{result:?}");
        let js = Command::new("node").arg(dir.join("dist/app.js")).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&js.stdout), "hello\n");
        assert!(std::fs::read(dir.join("dist/app.wasm")).unwrap().starts_with(b"\0asm"));
        assert!(std::fs::read(dir.join("dist/app")).unwrap().starts_with(b"\x7fELF"));
        assert_eq!(std::fs::metadata(dir.join("dist/app")).unwrap().permissions().mode() & 0o111, 0o111);
        assert_eq!(std::fs::metadata(dir.join("dist/app.js")).unwrap().permissions().mode() & 0o111, 0);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn hibana_build_scripts_generate_web_library_and_typescript() {
    let dir = std::env::temp_dir().join(format!("hibana-build-web-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/rules.hb"), "pub type Positive = int[x -> x > 0]\n").unwrap();
    std::fs::write(dir.join("src/app.hb"), r#"
use rules
pub fn validate(n: int) -> Result<rules::Positive, String> with {} {
    refine<rules::Positive>(n)
}
"#).unwrap();
    std::fs::write(dir.join("build.hb"), r#"
use std::build
use std::fs
use std::wasm_web

fn main() {
    let compiled = build::compile_wasm('src/app.hb', ['validate'])
    fs::write_bytes('dist/app.wasm', compiled.first)
    fs::write_bytes('dist/app.mjs', wasm_web::utf8(wasm_web::javascript(compiled.second)))
    fs::write_bytes('dist/app.d.mts', wasm_web::utf8(wasm_web::typescript(compiled.second)))
    fs::write_bytes('dist/utf8.txt', wasm_web::utf8('é日本語🌸'))
}
"#).unwrap();
    let build = Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg("build.hb").current_dir(&dir).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let declarations = std::fs::read_to_string(dir.join("dist/app.d.mts")).unwrap();
    assert!(declarations.contains(r#""validate": (arg0: number) => { ok: true; value: number } | { ok: false; error: string };"#));
    assert_eq!(std::fs::read(dir.join("dist/utf8.txt")).unwrap(), "é日本語🌸".as_bytes());
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
    for (src, name) in [
        ("use std::fs\nfn main() { fs::write_bytes('x', [1]) }", "fs.write_bytes"),
        ("use std::build\nfn main() { put(len(build::compile_wasm('src/app.hb', ['validate']).first)) }", "build.compile_wasm"),
    ] {
        for e in [hibana::to_js(src).unwrap_err(), hibana::to_wasm(src).unwrap_err(), hibana::to_native(src).unwrap_err()] {
            assert!(e.to_string().contains(&format!("{name} はインタプリタでの実行")), "{e}");
        }
    }
    assert!(hibana::to_js("use std::build\nfn f() -> Vec<int> { build::compile('a.hb', 'js') }\nfn main() {}").is_err());
    // 読み込んだだけで呼ばないスクリプト用の関数は、出力を妨げない
    assert!(hibana::to_js("use std::build\nfn main() { put(1) }").is_ok());
}

// 標準ライブラリの実装用の組み込みは、利用者のコードからは未定義の関数に見える
#[test]
fn library_only_builtins_are_hidden_from_user_code() {
    for name in ["num_floor(1.5)", "num_sqrt(4.0)", "compile_file('a.hb', 'js')", "compile_wasm_file('a.hb', ['f'])", "write_file('x', [1], true)"] {
        let source = format!("fn main() {{ {name} }}");
        let e = hibana::run(&source, &mut Vec::new()).unwrap_err();
        assert!(e.to_string().contains("未定義の関数"), "{name}: {e}");
    }
    // 名前が標準ライブラリと同じでも、利用者のファイルは標準ライブラリとして扱わない
    let dir = std::env::temp_dir().join(format!("hibana-library-only-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("math.hb"), "pub fn f() -> int with {} { num_floor(1.5) }\n").unwrap();
    let e = hibana::pipeline::load("use math\nfn main() { put(math::f()) }", &dir)
        .and_then(hibana::pipeline::parse)
        .and_then(|p| hibana::pipeline::check(p, hibana::pipeline::Kind::Executable))
        .map(|_| ())
        .unwrap_err();
    assert!(e.to_string().contains("未定義の関数 num_floor"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
    // ライブラリの関数を通せば使える。型検査が足す呼び出し（refine の num_floor）も通る
    let mut out = Vec::new();
    hibana::run("use std::math\nfn main() { put(math::floor(1.5))\n put(math::sqrt(4.0))\n put(refine<int>(3.0)) }", &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "1\n2\nstd.Ok(3)\n");
}
