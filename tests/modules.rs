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
    let mut program = files.parse("use dep\nfn main() { let p = dep::Point { value: dep::answer() }\n put(p.value)\n put(dep::Chosen(3)) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"42\ndep::Chosen(3)\n");
    for (name, expression) in [
        ("hidden", "hidden()"),
        ("Secret", "Secret { value: 1 }"),
        ("Hidden", "fn(n: Hidden) { n }"),
        ("PrivateValue", "PrivateValue"),
        ("Private", "fn(n: Private) { n }"),
        ("Internal", "fn(n: Internal<int>) { n }"),
    ] {
        for (selection, expression) in [
            (String::new(), expression.replacen(name, &format!("dep::{name}"), 1)),
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
    let mut program = files.parse("use dep\nfn main() { with dep::Log(fn(message) { put(message) }) { dep::exported() } }").unwrap();
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
fn inferred_requirements_still_require_matching_providers() {
    let declarations = r#"
pub type Digit = int[d -> 0 <= d && d <= 9]
pub type Convert = (int) -> String with Async
pub trait Source<T> { fn value(self) -> T with {} }
fn declared_digit() -> Digit with Source<Digit> { value() }
fn digit() -> Digit { declared_digit() }
fn forward_digit() -> Digit { digit() }
fn convert() -> Convert {
    let f: Convert = value()
    f
}
fn forward_convert() -> Convert { convert() }
"#;
    for (export, expected) in [
        ("pub fn entry() -> Digit with {} { forward_digit() }", "with dep::Source<int["),
        ("pub fn entry() -> Digit with Source<int> { forward_digit() }", "が必要だが"),
        ("pub fn entry() -> Convert with {} { forward_convert() }", "with dep::Source<(int) -> String with Async>"),
        ("pub fn entry() -> Convert with Source<(int) -> String> { forward_convert() }", "が必要だが"),
    ] {
        let dependency = format!("{declarations}\n{export}");
        let files = Files::new("inferred-provider", &[("dep.hb", &dependency)]);
        let mut program = files.parse("use dep\nfn main() {}").unwrap();
        let error = hibana::typeck::check(&mut program).unwrap_err();
        assert!(error.msg.contains(expected), "{export}: {error}");
        let source = error.source.unwrap();
        assert!(source.name.ends_with("dep.hb"));
        assert!(error.span.unwrap().0 >= declarations.len());
    }
}

#[test]
fn overlapping_trait_impls_point_to_both_declarations() {
    let trait_decl = "trait Mark<T> { fn mark(self) -> int }";
    for (first, second) in [
        ("impl Mark<int> for int", "impl Mark<int> for int"),
        ("impl<T> Mark<int> for Vec<T>", "impl Mark<int> for Vec<int>"),
        ("impl<T> Mark<int> for Vec<T>", "impl<T> Mark<T> for Vec<int>"),
    ] {
        let source = format!("{trait_decl}\n{first} {{ fn mark(self) -> int {{ 1 }} }}\n{second} {{ fn mark(self) -> int {{ 2 }} }}\nfn main() {{}}");
        let mut program = pipeline::parse(source.as_str()).unwrap();
        let error = hibana::typeck::check(&mut program).unwrap_err();
        assert!(error.msg.contains("トレイト実装が重複"), "{source}: {error}");
        let sites = [error.span.unwrap().0, error.origin.unwrap().0];
        assert_eq!(sites, [source.rfind(second).unwrap(), source.find(first).unwrap()]);
        assert_eq!(error.origin_source, Some(error.source_id));
    }
}

#[test]
fn trait_impl_overlaps_are_detected_across_modules() {
    let files = Files::new("overlapping-trait-impls", &[
        ("api.hb", "pub trait Mark { fn mark(self) -> int with {} }"),
        ("left.hb", "use api -> {Mark}\nimpl<T> Mark for Vec<T> { fn mark(self) -> int { 1 } }"),
        ("right.hb", "use api -> {Mark}\nimpl Mark for Vec<int> { fn mark(self) -> int { 2 } }"),
    ]);
    let mut program = files.parse("use left\nuse right\nfn main() {}").unwrap();
    let error = hibana::typeck::check(&mut program).unwrap_err();
    assert!(error.msg.contains("トレイト実装が重複"), "{error}");
    let sites = [
        (error.source.unwrap(), error.span.unwrap().0),
        (error.origin_file.unwrap(), error.origin.unwrap().0),
    ];
    assert!(sites.iter().any(|(source, _)| source.name.ends_with("left.hb")));
    assert!(sites.iter().any(|(source, _)| source.name.ends_with("right.hb")));
    assert!(sites.iter().all(|(source, at)| source.text[*at..].starts_with("impl")));

    let entry = files.0.join("main.hb");
    std::fs::write(&entry, "use left\nuse right\nfn main() {}").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg(entry)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    for expected in ["left.hb", "right.hb", "重複する実装の宣言"] {
        assert!(stderr.contains(expected), "{stderr}");
    }
}

#[test]
fn constrained_trait_impls_are_selected_independently_of_declaration_order() {
    let traits = "trait Eligible { fn eligible(self) -> bool }\ntrait Describe { fn describe(self) -> String }";
    let generic = "impl<A: Eligible> Describe for Vec<A> { fn describe(self) -> String { 'generic' } }";
    let concrete = "impl Describe for Vec<int> { fn describe(self) -> String { 'int' } }";
    let eligible = "impl Eligible for int { fn eligible(self) -> bool { true } }";
    let call = "fn call<A: Describe>(x: A) -> String { x.describe() }";

    for (first, second) in [(generic, concrete), (concrete, generic)] {
        let source = format!("{traits}\n{first}\n{second}\n{call}\nfn main() {{ put(call([1]))\nput([1].describe()) }}");
        let mut output = Vec::new();
        hibana::run(&source, &mut output).unwrap();
        assert_eq!(output, b"int\nint\n");

        for invocation in ["call([1])", "[1].describe()"] {
            let source = format!("{traits}\n{eligible}\n{first}\n{second}\n{call}\nfn main() {{ put({invocation}) }}");
            let error = hibana::run(&source, &mut Vec::new()).unwrap_err();
            assert!(error.msg.contains("トレイト実装が曖昧"), "{error}");
            assert_eq!(error.span.unwrap().0, source.rfind(invocation).unwrap());
            assert_eq!(error.related.len(), 2);
            for candidate in [generic, concrete] {
                let site = error.related.iter().find(|site| {
                    site.span.start.offset == source.find(candidate).unwrap()
                }).unwrap();
                assert_eq!(site.source.as_ref().unwrap().id, error.source_id);
            }
        }
    }
}

#[test]
fn ambiguous_trait_impls_report_the_call_and_both_modules() {
    let source = "use api\nuse generic\nuse concrete\nfn main() { put([1].describe()) }";
    let files = Files::new("ambiguous-trait-impls", &[
        ("api.hb", "pub trait Eligible { fn eligible(self) -> bool with {} }\npub trait Describe { fn describe(self) -> String with {} }\nimpl Eligible for int { fn eligible(self) -> bool { true } }"),
        ("generic.hb", "use api -> {Eligible, Describe}\nimpl<A: Eligible> Describe for Vec<A> { fn describe(self) -> String { 'generic' } }"),
        ("concrete.hb", "use api -> {Describe}\nimpl Describe for Vec<int> { fn describe(self) -> String { 'int' } }"),
        ("main.hb", source),
    ]);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg(files.0.join("main.hb"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    for expected in ["トレイト実装が曖昧", "main.hb", "generic.hb", "concrete.hb"] {
        assert!(stderr.contains(expected), "{stderr}");
    }
}

#[test]
fn same_named_trait_methods_are_ambiguous_regardless_of_trait_name() {
    for other in ["Alpha", "Zeta"] {
        let traits = format!(
            "trait {other} {{ fn label(self) -> String }}\ntrait Beta {{ fn label(self) -> String }}"
        );
        let other_impl = format!("impl {other} for int {{ fn label(self) -> String {{ 'other' }} }}");
        let beta_impl = "impl Beta for int { fn label(self) -> String { 'beta' } }";
        let main = "fn main() { put(1.label()) }";
        let source = format!("{traits}\n{other_impl}\n{beta_impl}\n{main}");
        let error = hibana::run(&source, &mut Vec::new()).unwrap_err();
        assert!(error.msg.contains("メソッド label が曖昧"), "{error}");
        for candidate in [other, "Beta"] {
            assert!(error.msg.contains(candidate), "{error}");
        }
        assert_eq!(error.span.unwrap().0, source.find("1.label()").unwrap());
        assert_eq!(error.related.len(), 2);
        for offset in traits.match_indices("fn label").map(|(offset, _)| offset) {
            assert!(error.related.iter().any(|site| {
                site.span.start.offset == offset
                    && site.source.as_ref().is_some_and(|source| source.id == error.source_id)
            }));
        }

        let source = format!("{traits}\n{beta_impl}\n{main}");
        let mut output = Vec::new();
        hibana::run(&source, &mut output).unwrap();
        assert_eq!(output, b"beta\n");
    }
}

#[test]
fn ambiguous_trait_methods_report_declarations_in_both_modules() {
    let files = Files::new("ambiguous-trait-methods", &[
        ("alpha.hb", "pub trait Label { fn label(self) -> String with {} }\nimpl Label for int { fn label(self) -> String { 'alpha' } }"),
        ("beta.hb", "pub trait Label { fn label(self) -> String with {} }\nimpl Label for int { fn label(self) -> String { 'beta' } }"),
        ("main.hb", "use alpha\nuse beta\nfn main() { put(1.label()) }"),
    ]);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg(files.0.join("main.hb"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    for expected in ["メソッド label が曖昧", "alpha::Label", "beta::Label", "main.hb", "alpha.hb", "beta.hb"] {
        assert!(stderr.contains(expected), "{stderr}");
    }
}

#[test]
fn ambiguous_equality_is_reported_instead_of_using_builtin_equality() {
    let source = "
use std -> {Eq}
impl Eq for Vec<int> { fn eq(self, other: Vec<int>) -> bool { false } }
fn main() { put([1] == [1]) }
";
    let error = hibana::run(source, &mut Vec::new()).unwrap_err();
    assert!(error.msg.contains("トレイト実装が曖昧"), "{error}");
    assert_eq!(error.related.len(), 2);
}

#[test]
fn the_expected_return_type_can_pick_a_trait_impl() {
    let source = "
trait Convert<T> { fn convert(self) -> T }
impl Convert<String> for int { fn convert(self) -> String { 'text' } }
impl Convert<bool> for int { fn convert(self) -> bool { true } }
fn main() {
    let text: String = 1.convert()
    let flag: bool = 1.convert()
    put(text)
    put(flag)
}
";
    let mut output = Vec::new();
    hibana::run(source, &mut output).unwrap();
    assert_eq!(output, b"text\ntrue\n");
}

#[test]
fn disjoint_trait_impls_are_usable_together() {
    let source = r#"
trait Mark<T> { fn mark(self) -> int }
impl Mark<int> for Vec<int> { fn mark(self) -> int { 1 } }
impl Mark<int> for Vec<String> { fn mark(self) -> int { 2 } }
impl Mark<String> for Vec<int> { fn mark(self) -> int { 3 } }
fn int_mark<A: Mark<int>>(value: A) -> int { value.mark() }
fn string_mark<A: Mark<String>>(value: A) -> int { value.mark() }

struct Pair<A, B> { left: A, right: B }
impl<A> Mark<int> for Pair<A, A> { fn mark(self) -> int { 4 } }
impl<B> Mark<int> for Pair<Vec<B>, B> { fn mark(self) -> int { 5 } }

fn main() {
    put(int_mark([1]))
    put(int_mark(['x']))
    put(string_mark([1]))
    put(int_mark(Pair { left: 1, right: 2 }))
    put(int_mark(Pair { left: [1], right: 2 }))
}
"#;
    let mut output = Vec::new();
    hibana::run(source, &mut output).unwrap();
    assert_eq!(output, b"1\n2\n3\n4\n5\n");
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
    for call in ["item.hidden()", "dep::Item::hidden(item)"] {
        let source = format!("use dep\nfn main() {{ let item = dep::Item {{ value: 1 }}\n put({call}) }}");
        let error = files.parse(&source).and_then(|mut program| hibana::typeck::check(&mut program).map(|_| ())).unwrap_err();
        assert!(error.msg.contains("非公開"), "{source}: {error}");
    }
    let mut program = files.parse("use dep\nfn main() { put(dep::Item { value: 1 }.get()) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
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
        "use std::math -> { floor }\nfn main() { put(math.sqrt(9)) }",
        "use std::math -> { floor }\nfn main() { put(math::sqrt(9)) }",
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
fn unqualified_calls_to_same_named_imported_functions_are_ambiguous() {
    let files = Files::new("ambiguous-functions", &[
        ("alpha.hb", "pub fn value() -> int with {} { 1 }"),
        ("beta.hb", "pub fn value() -> int with {} { 2 }"),
    ]);
    for (first, second) in [
        ("use alpha -> {value}", "use beta -> {value}"),
        ("use beta -> {value}", "use alpha -> {value}"),
        ("use alpha -> *", "use beta -> {value}"),
        ("use beta -> *", "use alpha -> *"),
    ] {
        let source = format!("{first}\n{second}\nfn main() {{ put(value()) }}");
        let error = files.parse(&source).unwrap_err();
        assert_eq!(error.msg, "関数 value が曖昧（alpha::value、beta::value）");
        assert_eq!(error.span.unwrap().0, source.find("value()").unwrap());
        assert_eq!(error.source.unwrap().text, source);
    }

    for (source, expected) in [
        ("use alpha\nuse beta\nfn main() { put(alpha::value())\nput(beta::value()) }", &b"1\n2\n"[..]),
        ("use alpha -> *\nuse alpha -> {value}\nfn main() { put(value()) }", &b"1\n"[..]),
        ("use alpha -> *\nuse beta -> *\nfn value() -> int { 3 }\nfn main() { put(value()) }", &b"3\n"[..]),
        ("use alpha -> *\nuse beta -> *\nfn main() { let value = fn() -> int { 4 }\nput(value()) }", &b"4\n"[..]),
    ] {
        let mut program = files.parse(source).unwrap();
        hibana::typeck::check(&mut program).unwrap();
        let mut output = Vec::new();
        hibana::eval::run(&program, &mut output).unwrap();
        assert_eq!(output, expected, "{source}");
    }
}

#[test]
fn explicit_type_arguments_do_not_disambiguate_imported_functions() {
    let files = Files::new("ambiguous-generic-functions", &[
        ("alpha.hb", "pub fn value<T>(x: T) -> T with {} { x }"),
        ("beta.hb", "pub fn value<T>(x: T) -> T with {} { x }"),
    ]);
    let source = "use alpha -> *\nuse beta -> *\nfn main() { put(value<int>(1)) }";
    let error = files.parse(source).expect_err("同名の関数が2つ見える");
    assert_eq!(error.msg, "関数 value が曖昧（alpha::value、beta::value）");
    assert_eq!(error.span.unwrap().0, source.find("value<int>").unwrap());

    let mut program = files.parse("use alpha\nuse beta -> *\nfn main() { put(alpha::value<int>(1))\nput(value<int>(2)) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"1\n2\n");
}

#[test]
fn string_imports_are_rejected_before_reading() {
    let files = Files::new("removed-string-imports", &[("util.hb", "pub fn value() -> int with {} { 42 }")]);
    for import in [
        "'./util.hb'",
        "'./util'",
        "'./util.js'",
        "'./util.mjs'",
        "\"./util.ts\"",
        "'some-package'",
        "'@scope/package'",
        "'node:path'",
    ] {
        for selection in ["", " -> {value}", " -> *"] {
            let source = format!("use {import}{selection}\nfn main() {{}}");
            for error in [
                pipeline::load(&source, &files.0).unwrap_err(),
                pipeline::parse(source.as_str()).unwrap_err(),
            ] {
                assert!(error.msg.contains("use の後にはモジュール名が必要"), "{source}: {}", error.msg);
                assert_eq!(error.source_id.0, 0);
                assert_eq!(error.span, Some(("use ".len(), import.len())));
                assert_eq!(error.source.unwrap().text, source);
            }
        }
    }
}

#[test]
fn builtin_imports_share_modules_and_preserve_source_text() {
    let source = "use std\nuse std -> {map}\nuse std::math\nuse std::math -> {floor}\nfn main() { put(map([1, 2], fn(x) { x + 1 }))\n put(std::Some(3))\n put(math::sqrt(9))\n put(floor(2.5)) }";
    let mut program = pipeline::parse(pipeline::load(source, std::path::Path::new(".")).unwrap()).unwrap();
    assert_eq!(program.modules.len(), 3);
    assert_eq!(program.modules[0].source.text, source);
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"[2, 3]\nSome(3)\n3\n2\n");
}

#[test]
fn text_macro_preserves_apostrophes() {
    let macros = r#"
macro Label ('<' ~ 'label' '>' body:Text '</' ~ 'label' ~ '>') {
    quote { ${str_of(body)} }
}
macro Literal ('literal' ~ value:String) {
    quote { $value }
}
"#;
    let files = Files::new("text-apostrophes", &[("labels.hb", macros)]);
    let source = r#"use labels
fn main() {
    put(<label>Don't use this</label>)
    put(literal'普通の文字列')
    put('Don\'t')
}
"#;
    assert_eq!(hibana::fmt::format(source).unwrap(), source);
    let program = files.parse(source).unwrap();
    let checked = pipeline::check(program, pipeline::Kind::Executable).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(checked.program(), &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "Don't use this\n普通の文字列\nDon't\n"
    );
    let error = files.parse("fn main() { put('closed')\n put('unclosed) }").unwrap_err();
    assert!(error.msg.contains("リテラルが閉じていない"), "{error}");
}

#[test]
fn named_imports_resolve_nested_paths_and_file_local_dependencies() {
    let files = Files::new("named-imports", &[
        ("util.hb", "pub fn twice(n: int) -> int with {} { n * 2 }"),
        ("tools/calc.hb", "use helper\npub fn answer() -> int with {} { helper::base() + 1 }"),
        ("tools/helper.hb", "pub fn base() -> int with {} { 41 }"),
    ]);
    for (selection, call) in [(" -> {answer}", "answer()"), (" -> *", "answer()"), ("\n -> {answer}", "answer()"), ("", "calc::answer()") ] {
        let source = format!("use util\nuse tools::calc{selection}\nuse tools::helper\nfn main() {{ put(util::twice({call})) }}");
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
    let mut program = files.parse("use std\nuse std::math\nfn main() { put(math::sqrt(9)) }").unwrap();
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
            ("a.hb", "use b\npub fn answer() -> int with {} { b::base() }"),
            ("b.hb", "use a\npub fn base() -> int with {} { 42 }"),
        ],
    );
    let mut program = files
        .parse("use a\nuse a\nfn main() { put(a::answer()) }")
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
    let source = "use types\nuse math\nuse macros\nfn main() { put(math::value(types::Point { x: twice 3 })) }";
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
    assert!(error.msg.contains("left::Item") && error.msg.contains("right::Item"), "{error}");
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
    assert!(error.msg.contains("Ambiguous macro match（a::pick, b::pick）"), "{error}");
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
            .parse("use dep\nfn main() { put(dep::value()) }")
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
        .parse("use dep\nfn main() { put(dep::value()) }")
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
    let mut program = files.parse("use one\nuse two\nfn main() { put(one::Shape::Node(3))\n put(two::Shape::Node(4, 5)) }").unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"one::Node(3)\ntwo::Node(4, 5)\n");
}

#[test]
fn effect_diagnostics_keep_the_call_and_operation_in_their_own_files() {
    let dependency = "pub trait Log { fn log(self, message: String) with {} }\npub fn trigger() with Log { log('entry') }";
    let files = Files::new("effect-origin", &[("dep.hb", dependency)]);
    let mut program = files
        .parse("use dep\nfn main() { dep::trigger() }")
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
        .parse("use generated\nfn main() { put(generated::answer()) }")
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
            ("a.hb", "use b\npub fn answer() -> int with {} { b::base() }"),
            ("b.hb", "pub fn base() -> int with {} { 42 }"),
        ],
    );
    // a.hb は b を修飾して使えるが、b を use していない main からは使えない
    let mut program = files
        .parse("use a\nfn main() { put(a::answer()) }")
        .unwrap();
    hibana::typeck::check(&mut program).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, b"42\n");
    let mut program = files
        .parse("use a\nfn main() { put(b::base()) }")
        .unwrap();
    let error = hibana::typeck::check(&mut program).err().unwrap();
    assert!(error.msg.contains("未定義"), "{}", error.msg);
}

#[test]
fn private_traits_cannot_be_provided_from_other_modules() {
    let files = Files::new("private-with", &[("dep.hb", "trait Internal { fn value(self) -> int }\npub fn one() -> int with {} { 1 }")]);
    for source in [
        "use dep\nfn main() { put(with dep::Internal(fn() { 3 }) { 1 }) }",
        "use dep -> *\nfn main() { put(with Internal(fn() { 3 }) { 1 }) }",
    ] {
        let error = files.parse(source).unwrap_err();
        assert!(error.msg.contains("非公開"), "{source}: {error}");
    }
}

#[test]
fn public_declarations_cannot_expose_private_types() {
    for (dependency, declaration) in [
        ("struct Secret { v: int }\npub struct Wrapper { s: Secret }", "dep::Wrapper"),
        ("struct Secret { v: int }\npub fn leak() -> Secret with {} { Secret { v: 1 } }", "dep::leak"),
        ("struct Secret { v: int }\npub fn take(xs: Vec<Secret>) -> int with {} { 0 }", "dep::take"),
        ("enum Secret { A }\npub enum Choice { Hidden(s: Secret) }", "dep::Choice"),
        ("type Secret = int\npub type Shown = Secret", "dep::Shown"),
        ("trait Internal { fn value(self) -> int }\npub fn need() -> int with Internal { Internal::value() }", "dep::need"),
    ] {
        let files = Files::new("private-signature", &[("dep.hb", dependency)]);
        let error = files.parse("use dep\nfn main() {}").unwrap_err();
        assert!(error.msg.contains(&format!("公開宣言 {declaration} に非公開型")), "{dependency}: {error}");
    }
}

#[test]
fn macros_use_private_names_with_the_permissions_of_their_module() {
    let library = "fn helper(x: int) -> int { x * 10 }\nstruct Secret { v: int }\nmacro boxed('boxed' x:Expr) { quote { helper(Secret { v: $x }.v) + { let s = Secret { v: 1 }\n s.v } } }";
    let files = Files::new("macro-permission", &[("lib.hb", library)]);
    let checked = pipeline::check(files.parse("use lib\nfn main() { let v = 2\n put(boxed v) }").unwrap(), pipeline::Kind::Executable).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(checked.program(), &mut output).unwrap();
    assert_eq!(output, b"21\n");
    let js = files.0.join("main.cjs");
    std::fs::write(&js, pipeline::emit_js(&checked).unwrap()).unwrap();
    let node = std::process::Command::new("node").arg(&js).output().unwrap();
    assert_eq!(node.stdout, b"21\n", "{}", String::from_utf8_lossy(&node.stderr));
    pipeline::lower(&checked, Some(&["main".into()])).unwrap();
    // 差し込んだ呼び出し側の式は、呼び出し側の権限のまま
    for source in [
        "use lib\nfn main() { put(boxed lib::helper(1)) }",
        "use lib\nfn main() { put(lib::helper(1)) }",
    ] {
        let error = files.parse(source).unwrap_err();
        assert!(error.msg.contains("lib::helper は非公開"), "{source}: {error}");
    }
}
