#[path = "common/wasm_web.rs"]
mod wasm_web;

use std::process::Command;

#[test]
fn json_numbers_match_f64_parsing_in_all_backends() {
    let mut inputs: Vec<String> = [
        "0.0", "-0e0", "0.1", "0.3", "-12.5", "1.25E+2", "-25e-2",
        "9007199254740992.0", "1.7976931348623157e308", "1.7976931348623158e308",
        "1.7976931348623159e308", "2.2250738585072014e-308", "2.225073858507201e-308",
        "5e-324", "2e-324", "3e-324", "2.4703282292062327e-324",
        "2.4703282292062328e-324", "-1e-999999999999999999999",
        "0e999999999999999999999", "1e999999999999999999999",
        "1.00000000000000011102230246251565404236316680908203125",
        "1.00000000000000011102230246251565404236316680908203126",
        "1.00000000000000033306690738754696212708950042724609375",
    ].into_iter().map(str::to_owned).collect();
    inputs.push(format!("0.{}1e401", "0".repeat(400)));
    inputs.push(format!("1{}.0e-400", "0".repeat(400)));
    let mut state = 919u64;
    for _ in 0..24 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let n = f64::from_bits(state);
        if n.is_finite() { inputs.push(format!("{n:.17e}")); }
    }
    let mut expected = String::new();
    for s in &inputs {
        let n: f64 = s.parse().unwrap();
        expected += &if n.is_finite() { format!("{n}\n") } else { "none\n".into() };
    }
    let invalid = ["00.1", "01e2", ".5", "1.", "1e", "1e+", "NaN", "Infinity"];
    inputs.extend(invalid.into_iter().map(str::to_owned));
    expected += &"none\n".repeat(invalid.len());
    let items = inputs.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ");
    let source = format!(
        "use std -> *\nfn main() {{\n for s in [{items}] {{\n put(match parse_json(s) {{ Some(v) -> match v {{ JNum(n) -> n.to_string() _ -> 'wrong variant' }} None -> 'none' }})\n }}\n}}",
    );
    let mut interpreted = Vec::new();
    hibana::run(&source, &mut interpreted).unwrap();
    assert_eq!(String::from_utf8(interpreted).unwrap(), expected);

    let dir = std::env::temp_dir().join(format!("hibana-json-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let web = wasm_web::build(&source, &["main".into()]).unwrap();
    std::fs::write(dir.join("json.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let runner = dir.join("run.mjs");
    std::fs::write(&runner, "import {readFileSync} from 'node:fs'; import {instantiate} from './glue.mjs'; const api = await instantiate(readFileSync(new URL('./json.wasm', import.meta.url))); api.main();").unwrap();
    let mut commands = vec![
        { let mut c = Command::new("node"); c.arg(runner); c },
        { let mut c = Command::new("node"); c.args(["-e", &hibana::to_js(&source).unwrap()]); c },
    ];
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("native");
        std::fs::write(&path, hibana::to_native(&source).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        commands.push(Command::new(path));
    }
    for c in &mut commands {
        let result = c.output().unwrap();
        assert!(result.status.success(), "{c:?}: {}\n{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{c:?}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
