use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn first_example_matches_all_backends() {
    let source = include_str!("../examples/first.hb");
    let dir = std::env::temp_dir().join(format!("hibana-first-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let js = dir.join("first.cjs");
    std::fs::write(&js, hibana::to_js(source).unwrap()).unwrap();
    let wasm = hibana::to_web(source, &["main".into()]).unwrap();
    std::fs::write(dir.join("first.wasm"), wasm.wasm).unwrap();
    std::fs::write(dir.join("first-glue.mjs"), wasm.javascript).unwrap();
    let wasm_runner = dir.join("first-wasm.mjs");
    std::fs::write(&wasm_runner, "import {readFileSync} from 'node:fs'; import {instantiate} from './first-glue.mjs'; const api = await instantiate(readFileSync(new URL('./first.wasm', import.meta.url))); api.main();").unwrap();
    let native = dir.join("first-native");
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&native, hibana::to_native(source).unwrap()).unwrap();
        std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let hb = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/first.hb");
    let prefix = "Hello, world!\n[section] start: math test\n20.248456731316587\n0\n[section] end: math test\nuid = 1\n100 / d = 20\nCircle at (0, 0), r=10\n[DEBUG] Empty shape\nuser1: 10\n[\"user2: 20\", \"user3: 30\"]\nWhat color do you like?\n";
    for (input, suffix) in [
        ("red\n7\n", "nice\nPick a digit\ndigit: 7\nnotified user1: 10\n"),
        ("blue\n12\n", "cool\nPick a digit\nerror: not a digit\nnotified user1: 10\n"),
        ("green\nno\n", "ok\nPick a digit\nerror: 整数として読めない\nnotified user1: 10\n"),
    ] {
        let mut commands = vec![Command::new(env!("CARGO_BIN_EXE_hibana")), Command::new("node"), Command::new("node")];
        commands[0].arg(&hb);
        commands[1].arg(&js);
        commands[2].arg(&wasm_runner);
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
        commands.push(Command::new(&native));
        for command in &mut commands {
            let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            assert_eq!(String::from_utf8(output.stdout).unwrap(), format!("{prefix}{suffix}"));
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
