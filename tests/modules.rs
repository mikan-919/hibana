use hibana::pipeline;
use std::path::PathBuf;

struct Files(PathBuf);

impl Files {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let directory =
            std::env::temp_dir().join(format!("hibana-modules-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for (path, text) in files {
            let path = directory.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Self(directory)
    }

    fn parse(&self, source: &str) -> hibana::error::R<hibana::parser::Program> {
        pipeline::parse(pipeline::load(source, &self.0)?)
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn distinct_files_with_the_same_module_name_are_rejected() {
    let files = Files::new(
        "collision",
        &[
            ("left/item.ts", "export function value(): number { return 1; }"),
            ("right/item.ts", "export function value(): number { return 1; }"),
        ],
    );
    let error = files
        .parse("use './left/item.ts'\nuse './right/item.ts'\nfn main() {}")
        .unwrap_err();
    assert!(error.msg.contains("モジュール名 item"));
    assert_eq!(error.source_id.0, 0);
}

#[test]
fn module_names_qualify_only_under_a_plain_use() {
    let files = Files::new("qualification", &[]);
    // -> { } と -> * は名前を取り込むだけで、モジュール名では修飾できない
    let mut program = files
        .parse("use std::math -> { floor }\nfn main() { put(floor(2.5)) }")
        .unwrap();
    hibana::typeck::check(&mut program).unwrap();
    for source in [
        "use std::math -> { floor }\nfn main() { put(math::sqrt(9)) }",
        "use std::math -> { floor }\nfn main() { put(math.sqrt(9)) }",
        "use std -> *\nfn main() { put(std.map([1], fn(x) { x })) }",
    ] {
        let mut program = files.parse(source).unwrap();
        let error = hibana::typeck::check(&mut program).err().unwrap();
        assert!(error.msg.contains("未定義"), "{source}: {}", error.msg);
    }
    // use 名前と use 名前 -> { } を両方書けば、どちらの形も使える
    let mut program = files
        .parse("use std::math\nuse std::math -> { floor }\nfn main() { put(math::sqrt(9))\n put(floor(2.5)) }")
        .unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"3\n2\n");
}

#[test]
fn hibana_file_imports_are_rejected_before_reading() {
    let files = Files::new("removed-hibana-imports", &[("util.hb", "fn value() -> int { 42 }")]);
    for path in ["./util.hb", "util.hb", "./util", "../util", "./missing.hb", "./missing", "./util.hb/../util.hb"] {
        for selection in ["", " -> {value}", " -> *"] {
            let source = format!("use '{path}'{selection}\nfn main() {{}}");
            for error in [
                pipeline::load(&source, &files.0).unwrap_err(),
                pipeline::parse(source.as_str()).unwrap_err(),
            ] {
                assert!(error.msg.contains("hibana ファイルは use で読み込めない"), "{source}: {}", error.msg);
                assert_eq!(error.source_id.0, 0);
                assert_eq!(error.span.unwrap().0, source.find('\'').unwrap());
                assert_eq!(error.source.unwrap().text, source);
            }
        }
    }
}

#[test]
fn builtin_imports_share_modules_and_preserve_source_text() {
    let source = "use std\nuse std -> {map}\nuse std::math\nuse std::math -> {floor}\nfn main() { put(map([1, 2], fn(x) { x + 1 }))\n put(std.Some(3))\n put(math.sqrt(9))\n put(floor(2.5)) }";
    let mut program = pipeline::parse(pipeline::load(source, std::path::Path::new(".")).unwrap()).unwrap();
    assert_eq!(program.modules.len(), 3);
    assert_eq!(program.modules[0].source.text, source);
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"[2, 3]\nSome(3)\n3\n2\n");
}

#[test]
fn typescript_import_forms_remain_available() {
    let files = Files::new("typescript-imports", &[("util.ts", "export function value(): number { return 42; }")]);
    for (selection, call) in [("", "util.value()"), (" -> {value}", "value()"), (" -> *", "value()")] {
        let source = format!("use './util.ts'{selection}\nfn main() {{ put({call}) }}");
        let program = files.parse(&source).unwrap();
        let checked = pipeline::check(program, pipeline::Kind::Executable).unwrap();
        pipeline::emit_js(&checked).unwrap();
    }
}
