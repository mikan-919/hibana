use hibana::parser::{NameKind, Program};
use std::process::Command;

#[test]
fn qualified_names_are_distinct_from_user_identifiers() {
    let tokens = hibana::lexer::lex("Point::get Point__get").unwrap();
    let hibana::lexer::Tok::Ident(qualified) = &tokens[0].tok else {
        panic!()
    };
    let hibana::lexer::Tok::Ident(plain) = &tokens[1].tok else {
        panic!()
    };
    assert!(matches!(qualified.kind, NameKind::Associated(..)));
    assert!(matches!(plain.kind, NameKind::User));
    assert_ne!(qualified, plain);
}

#[test]
fn structured_names_and_generated_roles_work_in_all_backends() {
    let source = r#"
struct Point { x: int }
struct Pair(int, int)
fn Point::get(self: Point) -> int { self.x }
fn Point__get(p: Point) -> int { p.x + 10 }
trait ReadValue {
    fn value(self) -> int
    fn double(self) -> int { self.value() + self.value() }
}
impl ReadValue for Point { fn value(self) -> int { self.x } }
fn extract<A: ReadValue>(x: A) -> int { x.double() }
fn main() {
    let point = Point { x: 4 }
    put(point.get())
    put(Point__get(point))
    put(extract(point))
    put(point)
    put(Pair(2, 3))
}
"#;
    let mut program: Program = hibana::parser::parse(source).unwrap();
    assert!(
        program
            .fns
            .iter()
            .any(|function| function.name.is_implementation())
    );
    assert!(
        program
            .fns
            .iter()
            .flat_map(|function| &function.params)
            .any(|(name, _)| name.is_instance())
    );
    let info = hibana::typeck::check(&mut program).unwrap();
    let expected = b"4\n14\n8\nPoint { x: 4 }\nPair(2, 3)\n";
    let mut output = Vec::new();
    hibana::eval::run(&program, &mut output).unwrap();
    assert_eq!(output, expected);
    let javascript = Command::new("node")
        .args(["-e", &hibana::js::emit(&program, &info)])
        .output()
        .unwrap();
    assert!(
        javascript.status.success(),
        "{}",
        String::from_utf8_lossy(&javascript.stderr)
    );
    assert_eq!(javascript.stdout, expected);

    let directory = std::env::temp_dir().join(format!("hibana-names-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let web = hibana::to_web(source, &["main".into()]).unwrap();
    std::fs::write(directory.join("program.wasm"), web.wasm).unwrap();
    std::fs::write(directory.join("glue.mjs"), web.javascript).unwrap();
    std::fs::write(directory.join("run.mjs"), "import fs from 'node:fs'; import {instantiate} from './glue.mjs'; (await instantiate(fs.readFileSync(new URL('./program.wasm', import.meta.url)))).main();").unwrap();
    let wasm = Command::new("node")
        .arg(directory.join("run.mjs"))
        .output()
        .unwrap();
    assert!(
        wasm.status.success(),
        "{}",
        String::from_utf8_lossy(&wasm.stderr)
    );
    assert_eq!(wasm.stdout, expected);
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let executable = directory.join("program");
        std::fs::write(&executable, hibana::to_native(source).unwrap()).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let native = Command::new(executable).output().unwrap();
        assert!(
            native.status.success(),
            "{}",
            String::from_utf8_lossy(&native.stderr)
        );
        assert_eq!(native.stdout, expected);
    }
    std::fs::remove_dir_all(directory).unwrap();
}
