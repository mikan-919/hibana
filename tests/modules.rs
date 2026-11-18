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
fn public_declarations_are_the_module_boundary() {
    let dependency = "fn hidden() -> int { 41 }\nstruct Secret { value: int }\ntype Hidden = int\nenum Private { PrivateValue }\ntrait Internal { fn value(self) -> int }\npub type Count = int\npub struct Point { value: Count }\npub enum Choice { Chosen(value: int) }\npub fn answer() -> int with {} { hidden() + 1 }";
    let files = Files::new("public-boundary", &[("dep.hb", dependency)]);
    let mut program = files.parse("use dep\nfn main() { let p = dep.Point { value: dep.answer() }\n put(p.value)\n put(dep.Chosen(3)) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"42\ndep.Chosen(3)\n");
    for (name, expression) in [
        ("hidden", "hidden()"),
        ("Secret", "Secret { value: 1 }"),
        ("Hidden", "fn(n: Hidden) { n }"),
        ("PrivateValue", "PrivateValue"),
        ("Private", "fn(n: Private) { n }"),
        ("Internal", "fn(n: Internal<int>) { n }"),
    ] {
        for (selection, expression) in [
            (String::new(), expression.replacen(name, &format!("dep.{name}"), 1)),
            (" -> *".into(), expression.into()),
            (format!(" -> {{{name}}}"), "0".into()),
        ] {
            let source = format!("use dep{selection}\nfn main() {{ let x = {expression} }}");
            let error = files.parse(&source).unwrap_err();
            assert!(error.msg.contains("非公開"), "{source}: {error}");
            assert_eq!(error.source_id.0, 0);
        }
    }
    let error = files.parse("use dep -> {Private::PrivateValue}\nfn main() {}").unwrap_err();
    assert!(error.msg.contains("非公開"), "{error}");
}

#[test]
fn public_functions_require_explicit_effects() {
    for source in [
        "pub fn value() -> int { 1 }",
        "pub fn value() {}",
        "extern { pub fn value() -> int }",
        "pub trait Value { fn value(self) -> int }",
    ] {
        let error = pipeline::parse(source).unwrap_err();
        assert!(error.msg.contains("with を明示"), "{source}: {error}");
        assert_eq!(error.span.unwrap().0, source.find("fn").unwrap());
    }
    let program = pipeline::parse("pub fn value() -> int with {} { 1 }").unwrap();
    let value = &program.fns[0];
    assert!(value.public && value.effects_explicit && value.effects.is_empty());
    let mut program = pipeline::parse("pub fn apply(f: () -> int with {}) -> int with {} { f() }\nfn main() { put(apply(fn() -> int { 1 })) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
}

#[test]
fn private_inference_stops_at_explicit_public_effects() {
    for visibility in ["", "pub "] {
        let dependency = format!("pub trait Log {{ fn log(self, message: String) with {{}} }}\nfn leaf() {{ log('entry') }}\nfn middle() {{ leaf() }}\n{visibility}fn exported() with {{}} {{ middle() }}");
        let files = Files::new(&format!("explicit-effects-{}", visibility.len()), &[("dep.hb", &dependency)]);
        let source = "use dep\nfn main() {}";
        let mut program = files.parse(source).unwrap();
        let error = hibana::typeck::check(&mut program).err().unwrap();
        assert!(error.msg.contains("with"), "{error}");
        assert!(error.source.unwrap().name.ends_with("dep.hb"));
        assert_eq!(error.span.unwrap().0, dependency.rfind("middle()").unwrap());
    }
    let dependency = "pub trait Log { fn log(self, message: String) with {} }\nfn leaf() { log('entry') }\nfn middle() { leaf() }\npub fn exported() with Log { middle() }";
    let files = Files::new("private-inference", &[("dep.hb", dependency)]);
    let mut program = files.parse("use dep\nfn main() { with dep.Log(fn(message) { put(message) }) { dep.exported() } }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
    for name in ["dep.leaf", "dep.middle", "dep.exported"] {
        let function = program.fns.iter().find(|f| f.name == name).unwrap();
        assert_eq!(function.effects.len(), 1, "{name}");
    }
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"entry\n");
}

#[test]
fn explicit_empty_effects_also_stop_lambda_inference() {
    for body in [
        "let f = fn() with {} { log('entry') }",
        "let f: () -> unit with Log = fn() with {} { log('entry') }",
        "consume(fn() with {} { log('entry') })",
    ] {
        let source = format!("trait Log {{ fn log(self, message: String) }}\nfn consume<R: Effect>(f: () -> unit with R) with R {{ f() }}\nfn main() {{ with Log(fn(message) {{ put(message) }}) {{ {body} }} }}");
        let error = pipeline::parse(source.as_str()).and_then(|mut program| hibana::typeck::check(&mut program).map(|_| ())).unwrap_err();
        assert!(error.msg.contains("with Log"), "{source}: {error}");
    }
}

#[test]
fn method_syntax_cannot_call_private_functions() {
    let dependency = "pub struct Item { value: int }\nfn Item::hidden(self: Item) -> int { self.value }\npub fn Item::get(self: Item) -> int with {} { self.hidden() }";
    let files = Files::new("private-methods", &[("dep.hb", dependency)]);
    for call in ["item.hidden()", "dep.Item::hidden(item)"] {
        let source = format!("use dep\nfn main() {{ let item = dep.Item {{ value: 1 }}\n put({call}) }}");
        let error = files.parse(&source).and_then(|mut program| hibana::typeck::check(&mut program).map(|_| ())).unwrap_err();
        assert!(error.msg.contains("非公開"), "{source}: {error}");
    }
    let mut program = files.parse("use dep\nfn main() { put(dep.Item { value: 1 }.get()) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
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
    let files = Files::new("removed-hibana-imports", &[("util.hb", "pub fn value() -> int with {} { 42 }")]);
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
        ("util.hb", "pub fn twice(n: int) -> int with {} { n * 2 }"),
        ("tools/calc.hb", "use helper\npub fn answer() -> int with {} { helper.base() + 1 }"),
        ("tools/helper.hb", "pub fn base() -> int with {} { 41 }"),
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
    let files = Files::new("named-collision", &[("left/item.hb", "pub fn value() with {} {}"), ("right/item.hb", "pub fn value() with {} {}")]);
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
            ("a.hb", "use b\npub fn answer() -> int with {} { b.base() }"),
            ("b.hb", "use a\npub fn base() -> int with {} { 42 }"),
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
            ("types.hb", "pub struct Point { x: int }"),
            (
                "math.hb",
                "use types -> *\npub fn value(p: Point) -> int with {} { p.x }",
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
fn syntax_and_macro_names_are_scoped_by_module() {
    let files = Files::new(
        "macro-names",
        &[
            ("left.hb", "syntax Item = number:Int\nmacro read('left' item:Item) { quote { $item.number } }"),
            ("right.hb", "syntax Item = word:Ident\nmacro read('right' item:Item) { quote { '${item.word}' } }"),
        ],
    );
    let mut program = files.parse(
        r#"
use left
use right
use left -> {Item}
use right -> *
syntax Item = value:String
macro read('here' item:Item) { quote { $item.value } }
macro both('both' l:left::Item r:right.Item) { quote { put($l.number)
 put('${r.word}') } }
fn main() { put(left 1)
 put(right apple)
 put(here 'local')
 both 2 pear }
"#,
    ).unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"1\napple\nlocal\n2\npear\n");

    let error = files.parse("use left -> {Item}\nuse right -> {Item}\nsyntax Both = Item").unwrap_err();
    assert!(error.msg.contains("left.Item") && error.msg.contains("right.Item"), "{error}");
    let error = files.parse("use left\nsyntax Wrong = Item").unwrap_err();
    assert!(error.msg.contains("未定義の規則 Item"), "{error}");
    files.parse("use left -> {Item}\nuse left -> *\nsyntax Imported = Item").unwrap();
}

#[test]
fn macro_candidates_follow_the_source_of_the_invocation() {
    let files = Files::new(
        "macro-scope",
        &[
            ("a.hb", "use hidden\nmacro pick('pick') { quote { 1 } }\nmacro wrap('wrap' value:Expr) { quote { hidden_value + $value } }\nfn wrap() -> int { 0 }"),
            ("b.hb", "macro pick('pick') { quote { 10 } }"),
            ("hidden.hb", "macro hidden_value('hidden_value') { quote { 100 } }"),
        ],
    );
    let mut program = files.parse(
        "use a -> {wrap}\nuse b -> {pick}\nfn main() { let hidden_value = 2\n put(wrap pick)\n put(hidden_value) }",
    ).unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"110\n2\n");

    let error = files.parse("use b\nuse a\nfn main() { put(pick) }").unwrap_err();
    assert!(error.msg.contains("Ambiguous macro match（a.pick, b.pick）"), "{error}");
}

#[test]
fn imported_parse_and_type_errors_keep_their_file_and_local_offsets() {
    for (name, text, expected) in [
        ("lex", "pub fn value() -> int with {} { /*", "/*"),
        ("parse", "pub fn value() -> int with {} { @ }", "@"),
        ("type", "pub fn value() -> int with {} {\n false\n}", "false"),
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
    let text = "pub fn value() -> int with {} {\n panic('stop')\n}";
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
            ("one.hb", "pub enum Shape { Node(value: int) }"),
            ("two.hb", "pub enum Shape { Node(left: int, right: int) }"),
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
    let dependency = "pub trait Log { fn log(self, message: String) with {} }\npub fn trigger() with Log { log('entry') }";
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
            "install\nuse generator -> {install}",
        ), (
            "generator.hb",
            "macro install('install') -> Decl { let exported = ident('answer')\n quote { fn hidden() -> int { 42 }\n pub fn $exported() -> int with {} { hidden() } } }",
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
            ("a.hb", "use b\npub fn answer() -> int with {} { b.base() }"),
            ("b.hb", "pub fn base() -> int with {} { 42 }"),
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

#[test]
fn private_traits_cannot_be_provided_from_other_modules() {
    let files = Files::new("private-with", &[("dep.hb", "trait Internal { fn value(self) -> int }\npub fn one() -> int with {} { 1 }")]);
    for source in [
        "use dep\nfn main() { put(with dep.Internal(fn() { 3 }) { 1 }) }",
        "use dep -> *\nfn main() { put(with Internal(fn() { 3 }) { 1 }) }",
    ] {
        let error = files.parse(source).unwrap_err();
        assert!(error.msg.contains("非公開"), "{source}: {error}");
    }
}

#[test]
fn public_declarations_cannot_expose_private_types() {
    for (dependency, declaration) in [
        ("struct Secret { v: int }\npub struct Wrapper { s: Secret }", "dep.Wrapper"),
        ("struct Secret { v: int }\npub fn leak() -> Secret with {} { Secret { v: 1 } }", "dep.leak"),
        ("struct Secret { v: int }\npub fn take(xs: Vec<Secret>) -> int with {} { 0 }", "dep.take"),
        ("enum Secret { A }\npub enum Choice { Hidden(s: Secret) }", "dep.Choice"),
        ("type Secret = int\npub type Shown = Secret", "dep.Shown"),
        ("trait Internal { fn value(self) -> int }\npub fn need() -> int with Internal { Internal::value() }", "dep.need"),
    ] {
        let files = Files::new("private-signature", &[("dep.hb", dependency)]);
        let error = files.parse("use dep\nfn main() {}").unwrap_err();
        assert!(error.msg.contains(&format!("公開宣言 {declaration} に非公開型")), "{dependency}: {error}");
    }
}
