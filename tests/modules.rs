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
fn string_hibana_imports_are_rejected_before_reading() {
    let files = Files::new("removed-hibana-imports", &[("util.hb", "fn value() -> int { 42 }")]);
    for path in ["./util.hb", "util.hb", "./util", "../util", "./missing.hb", "./missing", "./util.hb/../util.hb"] {
        for selection in ["", " -> {value}", " -> *"] {
            let source = format!("use '{path}'{selection}\nfn main() {{}}");
            for error in [
                pipeline::load(&source, &files.0).unwrap_err(),
                pipeline::parse(source.as_str()).unwrap_err(),
            ] {
                assert!(error.msg.contains("hibana ファイルは文字列の経路で読み込めない"), "{source}: {}", error.msg);
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

#[test]
fn named_imports_resolve_nested_paths_and_file_local_dependencies() {
    let files = Files::new("named-imports", &[
        ("util.hb", "fn twice(n: int) -> int { n * 2 }"),
        ("tools/calc.hb", "use helper\nfn answer() -> int { helper.base() + 1 }"),
        ("tools/helper.hb", "fn base() -> int { 41 }"),
    ]);
    for (selection, call) in [("", "calc.answer()"), (" -> {answer}", "answer()"), (" -> *", "answer()"), ("\n -> {answer}", "answer()"), ("", "calc::answer()") ] {
        let source = format!("use util\nuse tools::calc{selection}\nuse tools::helper\nfn main() {{ put(util.twice({call})) }}");
        let mut program = files.parse(&source).unwrap();
        assert_eq!(program.modules.len(), 4);
        hibana::typeck::check(&mut program).unwrap();
        let mut output = Vec::new();
        hibana::eval::run(&program, &mut output).unwrap();
        assert_eq!(output, b"84\n");
    }
}

#[test]
fn builtin_imports_take_precedence_over_local_files() {
    let files = Files::new("builtin-precedence", &[("std.hb", "@"), ("std/math.hb", "@")]);
    let mut program = files.parse("use std\nuse std::math\nfn main() { put(math.sqrt(9)) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"3\n");
}

#[test]
fn named_files_with_the_same_module_name_are_rejected() {
    let files = Files::new("named-collision", &[("left/item.hb", "fn value() {}"), ("right/item.hb", "fn value() {}")]);
    let source = "use left::item\nuse right::item\nfn main() {}";
    let error = files.parse(source).unwrap_err();
    assert!(error.msg.contains("モジュール名 item"));
    assert_eq!(error.span.unwrap().0, source.find("right::item").unwrap());
    assert_eq!(error.source.unwrap().text, source);
}

#[test]
fn cyclic_and_repeated_named_imports_share_modules() {
    let files = Files::new(
        "cycles",
        &[
            ("a.hb", "use b\nfn answer() -> int { b.base() }"),
            ("b.hb", "use a\nfn base() -> int { 42 }"),
        ],
    );
    let mut program = files
        .parse("use a\nuse a\nfn main() { put(a.answer()) }")
        .unwrap();
    assert_eq!(program.modules.len(), 3);
    assert_eq!(program.modules[1].name, "a");
    assert_eq!(program.modules[2].name, "b");
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"42\n");
}

#[test]
fn imported_macros_and_types_share_declarations_without_source_concatenation() {
    let files = Files::new(
        "declarations",
        &[
            ("types.hb", "struct Point { x: int }"),
            (
                "math.hb",
                "use types -> *\nfn value(p: Point) -> int { p.x }",
            ),
            (
                "macros.hb",
                "macro twice('twice' x:Int) { quote { $x + $x } }",
            ),
        ],
    );
    let source = "use types\nuse math\nuse macros\nfn main() { put(math.value(types.Point { x: twice 3 })) }";
    let mut program = files.parse(source).unwrap();
    assert_eq!(program.modules[0].source.text, source);
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"6\n");
}

#[test]
fn imported_parse_and_type_errors_keep_their_file_and_local_offsets() {
    for (name, text, expected) in [
        ("lex", "fn value() -> int { /*", "/*"),
        ("parse", "fn value() -> int { @ }", "@"),
        ("type", "fn value() -> int {\n false\n}", "false"),
    ] {
        let files = Files::new(name, &[("dep.hb", text)]);
        let result = files
            .parse("use dep\nfn main() { put(dep.value()) }")
            .and_then(|program| pipeline::check(program, pipeline::Kind::Executable));
        let error = result.err().unwrap();
        let source = error.source.unwrap();
        assert!(source.name.ends_with("dep.hb"), "{}", source.name);
        assert_eq!(source.text, text);
        assert_eq!(error.span.unwrap().0, text.find(expected).unwrap());
    }
}

#[test]
fn imported_runtime_errors_keep_their_file() {
    let text = "fn value() -> int {\n panic('stop')\n}";
    let files = Files::new("runtime", &[("dep.hb", text)]);
    let mut program = files
        .parse("use dep\nfn main() { put(dep.value()) }")
        .unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let error = hibana::eval::run(&program, &mut Vec::new()).unwrap_err();
    assert!(error.source.unwrap().name.ends_with("dep.hb"));
    assert_eq!(error.span.unwrap().0, text.find("panic").unwrap());
}

#[test]
fn same_named_constructors_resolve_through_their_type_and_module() {
    let files = Files::new(
        "constructors",
        &[
            ("one.hb", "enum Shape { Node(value: int) }"),
            ("two.hb", "enum Shape { Node(left: int, right: int) }"),
        ],
    );
    let mut program = files.parse("use one\nuse two\nfn main() { put(one.Shape::Node(3))\n put(two.Shape::Node(4, 5)) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"one.Node(3)\ntwo.Node(4, 5)\n");
}

#[test]
fn effect_diagnostics_keep_the_call_and_operation_in_their_own_files() {
    let dependency = "trait Log { fn log(self, message: String) }\nfn trigger() with Log { log('entry') }";
    let files = Files::new("effect-origin", &[("dep.hb", dependency)]);
    let mut program = files
        .parse("use dep\nfn main() { dep.trigger() }")
        .unwrap();
    let error = hibana::typeck::check(&mut program).err().unwrap();
    assert_eq!(error.source_id.0, 0);
    assert_ne!(error.origin_source.unwrap(), error.source_id);
    assert!(error.origin_file.unwrap().name.ends_with("dep.hb"));
    assert_eq!(
        error.origin.unwrap().0,
        dependency.find("log('entry')").unwrap()
    );
}

#[test]
fn declaration_macros_keep_generated_names_in_imported_modules() {
    let files = Files::new(
        "generated-declarations",
        &[(
            "generated.hb",
            "macro install('install') -> Decl { let exported = ident('answer')\n quote { fn hidden() -> int { 42 }\n fn $exported() -> int { hidden() } } }\ninstall",
        )],
    );
    let mut program = files
        .parse("use generated\nfn main() { put(generated.answer()) }")
        .unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"42\n");
}

#[test]
fn module_names_stay_in_the_file_that_uses_them() {
    let files = Files::new(
        "file-scope",
        &[
            ("a.hb", "use b\nfn answer() -> int { b.base() }"),
            ("b.hb", "fn base() -> int { 42 }"),
        ],
    );
    // a.hb は b を修飾して使えるが、b を use していない main からは使えない
    let mut program = files
        .parse("use a\nfn main() { put(a.answer()) }")
        .unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"42\n");
    let mut program = files
        .parse("use a\nfn main() { put(b.base()) }")
        .unwrap();
    let error = hibana::typeck::check(&mut program).err().unwrap();
    assert!(error.msg.contains("未定義"), "{}", error.msg);
}
