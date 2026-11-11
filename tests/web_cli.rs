use std::process::Command;

#[test]
fn web_cli_emits_pair_and_runs_generated_module() {
    let dir = std::env::temp_dir().join(format!("hibana-web-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("hello.web.hb");
    std::fs::write(
        &source,
        "fn echo(s: str) -> str { s }\nfn answer(n: int) -> int { n + 1 }\n",
    )
    .unwrap();
    let out = dir.join("artifacts");
    let result = Command::new(env!("CARGO_BIN_EXE_hibana"))
        .arg("web")
        .arg(&source)
        .args(["--export", "echo", "--export", "answer", "--out-dir"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stdout.is_empty());
    assert!(out.join("hello.web.wasm").exists());
    assert!(out.join("hello.web.mjs").exists());
    let script = dir.join("check.mjs");
    std::fs::write(&script, r#"
import { readFile } from 'node:fs/promises';
import { instantiate } from './artifacts/hello.web.mjs';
const api = await instantiate(await readFile(new URL('./artifacts/hello.web.wasm', import.meta.url)));
if (api.echo('日本語🌸') !== '日本語🌸' || api.answer(41) !== 42n) throw Error('結果が違う');
"#).unwrap();
    let result = Command::new("node").arg(script).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn web_cli_requires_exports_and_rejects_incomplete_options() {
    for options in [
        vec![],
        vec!["--export"],
        vec!["--export", "echo", "--out-dir"],
        vec!["--wat"],
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_hibana"))
            .args(["web", "missing.hb"])
            .args(options)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(!result.stderr.is_empty());
    }
}

#[test]
fn web_api_rejects_empty_exports() {
    assert!(hibana::to_web("fn echo(s: str) -> str { s }", &[]).is_err());
}
