use hibana::pipeline::{self, Kind};
use std::path::PathBuf;

struct Files(PathBuf);

impl Files {
    fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let directory =
            std::env::temp_dir().join(format!("hibana-modules-{}-{name}", std::process::id()));
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
fn cyclic_imports_and_alternate_paths_share_modules() {
    let files = Files::new(
        "cycles",
        &[
            ("a.hb", "use './b.hb'\nfn answer() -> int { b.base() }"),
            ("b.hb", "use './a.hb'\nfn base() -> int { 42 }"),
        ],
    );
    let mut program = files
        .parse("use './a.hb'\nuse './a'\nfn main() { put(a.answer()) }")
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
                "use './types.hb' -> *\nfn value(p: Point) -> int { p.x }",
            ),
            (
                "syntax.hb",
                "macro twice('twice' x:Int) { quote { $x + $x } }",
            ),
        ],
    );
    let source = "use './types.hb'\nuse './math.hb'\nuse './syntax.hb'\nfn main() { put(math.value(types.Point { x: twice 3 })) }";
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
            .parse("use './dep.hb'\nfn main() { put(dep.value()) }")
            .and_then(|program| pipeline::check(program, Kind::Executable));
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
        .parse("use './dep.hb'\nfn main() { put(dep.value()) }")
        .unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let error = hibana::eval::run(&program, &mut Vec::new()).unwrap_err();
    assert!(error.source.unwrap().name.ends_with("dep.hb"));
    assert_eq!(error.span.unwrap().0, text.find("panic").unwrap());
}

#[test]
fn distinct_files_with_the_same_module_name_are_rejected() {
    let files = Files::new(
        "collision",
        &[
            ("left/item.hb", "fn value() {}"),
            ("right/item.hb", "fn value() {}"),
        ],
    );
    let error = files
        .parse("use './left/item.hb'\nuse './right/item.hb'\nfn main() {}")
        .unwrap_err();
    assert!(error.msg.contains("モジュール名 item"));
    assert_eq!(error.source_id.0, 0);
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
    let mut program = files.parse("use './one.hb'\nuse './two.hb'\nfn main() { put(one.Shape::Node(3))\n put(two.Shape::Node(4, 5)) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"one.Node(3)\ntwo.Node(4, 5)\n");
}

#[test]
fn effect_diagnostics_keep_the_call_and_operation_in_their_own_files() {
    let dependency = "effect Log { fn log(message: String) }\nfn trigger() ! Log { log('entry') }";
    let files = Files::new("effect-origin", &[("dep.hb", dependency)]);
    let mut program = files
        .parse("use './dep.hb'\nfn main() { dep.trigger() }")
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
