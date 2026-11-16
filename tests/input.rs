use std::io::{Read, Write};
use std::process::{Command, Output, Stdio};

fn interact(command: &mut Command, bytes: &[u8]) -> Output {
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    // 入力を渡す前に案内が表示されることも確かめる。
    let mut prompt = [0; 7];
    child.stdout.as_mut().unwrap().read_exact(&mut prompt).unwrap();
    assert_eq!(&prompt, b"prompt\n");
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let mut output = child.wait_with_output().unwrap();
    output.stdout.splice(..0, prompt);
    output
}

#[test]
fn input_lines_match_between_interpreter_and_javascript() {
    let dir = std::env::temp_dir().join(format!("hibana-input-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = "fn main() { put('prompt')\n put('[' + input() + ']')\n put('[' + input() + ']')\n put('[' + input() + ']')\n put('[' + input() + ']')\n put('[' + input() + ']')\n put('[' + input() + ']') }";
    let hb = dir.join("input.hb");
    std::fs::write(&hb, source).unwrap();
    let js = hibana::to_js(source).unwrap();
    let input = "赤🌸\n\nblue\r\n\u{feff}keep\nlast\r";
    let expected = "prompt\n[赤🌸]\n[]\n[blue]\n[\u{feff}keep]\n[last\r]\n[]\n";
    let mut commands = vec![{
        let mut command = Command::new(env!("CARGO_BIN_EXE_hibana"));
        command.arg(&hb);
        command
    }];
    for extension in ["cjs", "mjs"] {
        let path = dir.join(format!("input.{extension}"));
        std::fs::write(&path, &js).unwrap();
        let mut command = Command::new("node");
        command.arg(path);
        commands.push(command);
    }
    for command in &mut commands {
        let output = interact(command, input.as_bytes());
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        let invalid = interact(command, &[0xff, b'\n']);
        assert!(!invalid.status.success());
    }
    assert!(hibana::run("fn main() { input(1) }", &mut Vec::new()).is_err());
    // 標準入力の取り込みを持たない出力先では、生成時に失敗させる。
    assert!(hibana::to_wasm(source).is_err());
    assert!(hibana::to_native(source).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
