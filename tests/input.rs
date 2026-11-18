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

fn assert_input_error(command: &Command, output: Output, stdout: &[u8], message: &str) {
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{command:?}: {stderr}");
    assert_eq!(output.stdout, stdout, "{command:?}");
    let report = stderr.lines().find(|line| !line.trim().is_empty())
        .and_then(|line| line.split_once("実行時エラー: "))
        .map(|(_, message)| message.split('（').next().unwrap());
    assert_eq!(report, Some(message), "{command:?}: {stderr}");
    assert_eq!(stderr.matches("実行時エラー:").count(), 1, "{command:?}: {stderr}");
}

#[test]
fn input_lines_match_all_backends() {
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
    let web = hibana::to_web(source, &["main".into()]).unwrap();
    std::fs::write(dir.join("input.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("input-glue.mjs"), web.javascript).unwrap();
    let runner = dir.join("input-wasm.mjs");
    std::fs::write(&runner, r#"
import {readFileSync} from 'node:fs';
import {instantiate} from './input-glue.mjs';
const api = await instantiate(readFileSync(new URL('./input.wasm', import.meta.url)));
try { api.main(); } catch (e) { console.error(e.message); process.exitCode = 1; }
"#).unwrap();
    let mut command = Command::new("node");
    command.arg(runner);
    commands.push(command);
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("input-native");
        std::fs::write(&path, hibana::to_native(source).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        commands.push(Command::new(path));
    }
    for command in &mut commands {
        let output = interact(command, input.as_bytes());
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stderr.is_empty(), "{command:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        let long = format!("{}🌸{}", "x".repeat(4095), "y".repeat(4092));
        let output = interact(command, format!("{long}\r\n").as_bytes());
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.stderr.is_empty(), "{command:?}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("prompt\n[{long}]\n[]\n[]\n[]\n[]\n[]\n"));
        for invalid_bytes in [&[0xff, b'\n'][..], &[0xc0, 0xaf, b'\n'], &[0xed, 0xa0, 0x80, b'\n'], &[0xf4, 0x90, 0x80, 0x80, b'\n'], &[0xe3, b'\n']] {
            let input = [b"valid\n", invalid_bytes, b"after\n"].concat();
            let output = interact(command, &input);
            assert_input_error(command, output, b"prompt\n[valid]\n", "標準入力の文字コードが UTF-8 でない");
        }
        #[cfg(target_os = "linux")]
        {
            // ディレクトリは開けるが、標準入力からの読み取りは失敗する。
            let output = command.stdin(std::fs::File::open(&dir).unwrap()).output().unwrap();
            assert_input_error(command, output, b"prompt\n", "標準入力を読めません");
        }
    }
    assert!(hibana::run("fn main() { input(1) }", &mut Vec::new()).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wasm_input_uses_host_callback() {
    let dir = std::env::temp_dir().join(format!("hibana-input-host-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let web = hibana::to_web("fn next_line() -> String { input() }", &["next_line".into()]).unwrap();
    std::fs::write(dir.join("input.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let script = dir.join("check.mjs");
    std::fs::write(&script, r#"
import {readFileSync} from 'node:fs';
import {instantiate} from './glue.mjs';
const bytes = new TextEncoder().encode('\ufeff日本🌸\r\n\nlast');
let pos = 0, fail = false;
const api = await instantiate(readFileSync(new URL('./input.wasm', import.meta.url)), {hibana: {read(fd, target) {
  if (fd !== 0) throw Error('標準入力でない');
  if (fail) return -1;
  const n = Math.min(target.length, bytes.length - pos);
  target.set(bytes.subarray(pos, pos + n)); pos += n; return n;
}}});
for (const expected of ['\ufeff日本🌸', '', 'last', '']) if (api.next_line() !== expected) throw Error('入力の結果が違う');
fail = true;
let message;
try { api.next_line(); } catch (e) { message = e.message; }
if (message !== '実行時エラー: 標準入力を読めません') throw Error('読み取りの失敗報告が違う: ' + message);
"#).unwrap();
    let output = Command::new("node").arg(script).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(dir).unwrap();
}
