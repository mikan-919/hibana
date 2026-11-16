use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn first_example_matches_between_interpreter_and_javascript() {
    let source = include_str!("../examples/first.hb");
    let dir = std::env::temp_dir().join(format!("hibana-first-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let js = dir.join("first.cjs");
    std::fs::write(&js, hibana::to_js(source).unwrap()).unwrap();
    let hb = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/first.hb");
    let prefix = "Hello, world!\n[section] start: math test\n20.248456731316587\n0\n[section] end: math test\nuid = 1\n100 / d = 20\nCircle at (0, 0), r=10\n[DEBUG] Empty shape\nWhat color do you like?\n";
    for (input, suffix) in [
        ("red\n7\n", "nice\nPick a digit\ndigit: 7\n"),
        ("blue\n12\n", "cool\nPick a digit\nerror: not a digit\n"),
        ("green\nno\n", "ok\nPick a digit\nerror: 整数として読めない\n"),
    ] {
        for (program, path) in [(env!("CARGO_BIN_EXE_hibana"), &hb), ("node", &js)] {
            let mut child = Command::new(program).arg(path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{prefix}{suffix}"));
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
