use std::process::Command;

#[test]
fn async_annotations_survive_cloning_and_numeric_elaboration() {
    let source = r#"
extern '' { fn delay(n: int) -> int ! Async }
fn apply<E: Effect>(f: () -> int ! E) -> int ! E { f() }
fn main() ! Async {
    let result: num = apply(fn() { delay(7) })
    put(result)
    put(all([fn() { delay(2) }, fn() { delay(3) }]))
}
"#;
    let mut program = hibana::parser::parse(source).unwrap();
    let info = hibana::typeck::check(&mut program).unwrap();
    let mut moved = program.clone();
    moved.fns.reserve(100);
    drop(program);
    let javascript = hibana::js::emit(&moved, &info);
    let output = Command::new("node")
        .args(["-e", &format!("globalThis.delay = async n => n;\n{javascript}")])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.stdout, b"7\n[2, 3]\n");
}
