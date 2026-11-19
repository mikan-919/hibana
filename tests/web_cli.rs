#[path = "common/wasm_web.rs"]
mod wasm_web;

use std::process::Command;

#[test]
fn web_api_emits_pair_and_runs_generated_module() {
    let dir = std::env::temp_dir().join(format!("hibana-web-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("hello.web.hb");
    std::fs::write(
        &source,
        "fn echo(s: String) -> String { s }\nfn answer(n: int) -> int { n + 1 }\n",
    )
    .unwrap();
    let out = dir.join("artifacts");
    let source = std::fs::read_to_string(&source).unwrap();
    let web = wasm_web::build(&source, &["echo".into(), "answer".into()]).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("hello.web.wasm"), web.wasm).unwrap();
    std::fs::write(out.join("hello.web.mjs"), web.javascript).unwrap();
    let script = dir.join("check.mjs");
    std::fs::write(&script, r#"
import { readFile } from 'node:fs/promises';
import { instantiate } from './artifacts/hello.web.mjs';
const api = await instantiate(await readFile(new URL('./artifacts/hello.web.wasm', import.meta.url)));
if (api.echo('日本語🌸') !== '日本語🌸' || api.answer(41) !== 42) throw Error('結果が違う');
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
fn removed_codegen_commands_are_rejected() {
    let dir = std::env::temp_dir().join(format!("hibana-removed-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("input.hb"), "fn main() { put(42) }").unwrap();
    for command in ["ir", "wasm", "glue", "web", "js", "c", "run"] {
        // A same-named file must not revive the removed command through implicit eval.
        std::fs::write(dir.join(command), "fn main() { put(42) }").unwrap();
        for args in [
            vec![command],
            vec![command, "input.hb"],
            vec![command, "input.hb", "--export", "main", "--out-dir", "dist"],
        ] {
            let result = Command::new(env!("CARGO_BIN_EXE_hibana"))
                .current_dir(&dir)
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(2), "{args:?}: {result:?}");
            assert!(result.stdout.is_empty(), "{args:?}");
            assert!(!result.stderr.is_empty(), "{args:?}");
            assert!(!dir.join("dist").exists());
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wasm_abi_can_describe_an_empty_export_set() {
    let (_, abi) = hibana::compile_wasm_library("fn echo(s: String) -> String { s }", &[]).unwrap();
    let abi: serde_json::Value = serde_json::from_str(&abi).unwrap();
    assert_eq!(abi["exports"], serde_json::json!([]));
}
