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
fn tests_require_explicit_files_and_named_imports_require_builtins() {
    let result = Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg("test")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("ファイル"));
    let error = hibana::parser::with_uses("use missing_library\nfn main() {}").unwrap_err();
    assert!(error
        .to_string()
        .contains("ライブラリ missing_library が無い"));
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
        "write_text", "write_bytes", "compile_file", "inspect_file_exports", "js_glue",
    ] {
        let source = format!("fn main() {{ {name}() }}");
        assert!(hibana::run(&source, &mut Vec::new()).is_err(), "{name}");
        let source = format!("fn {name}() -> int {{ 42 }} fn main() {{ put({name}()) }}");
        let mut output = Vec::new();
        hibana::run(&source, &mut output).unwrap();
        assert_eq!(output, b"42\n", "{name}");
    }
}
