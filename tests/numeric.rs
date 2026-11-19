#[path = "common/wasm_web.rs"]
mod wasm_web;

use std::process::Command;

fn literal(n: f64) -> String {
    let s = n.to_string();
    if s.contains('.') { s } else { s + ".0" }
}

#[test]
fn compiled_numbers_match_interpreter_at_float_boundaries() {
    let mut numbers = vec![0.0, -0.0, f64::MIN_POSITIVE, f64::from_bits(1), f64::from_bits(2), f64::from_bits((1 << 52) - 1), f64::MAX, -f64::MAX, 0.1, 1.0, f64::from_bits(1.0f64.to_bits() - 1), f64::from_bits(1.0f64.to_bits() + 1)];
    let mut state = 919u64;
    for _ in 0..160 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let n = f64::from_bits(state);
        if n.is_finite() { numbers.push(n); }
    }
    let mut source = String::from("use std::math\nfn as_number(value: number) -> number { value }\nfn as_num(value: number) -> number { value }\nfn main() {\n");
    let mut expected = String::new();
    for n in numbers {
        source += &format!("put(as_num(as_number({})))\n", literal(n));
        expected += &format!("{n}\n");
    }
    for (a, b) in [(f64::MAX, 3.0), (f64::MAX, f64::from_bits(1)), (-5.5, 2.0), (f64::MIN_POSITIVE, f64::from_bits(3)), (1.0, -0.5)] {
        source += &format!("put({} % {})\n", literal(a), literal(b));
        expected += &format!("{}\n", a % b);
    }
    for exponent in [3, 10, -10] {
        source += &format!("put(1.1 ** {})\n", literal(exponent as f64));
        let mut factor = if exponent < 0 { 1.0 / 1.1 } else { 1.1 };
        let mut n = exponent;
        let mut result = 1.0;
        while n != 0 { if n % 2 != 0 { result *= factor; } n /= 2; if n != 0 { factor *= factor; } }
        expected += &format!("{result}\n");
    }
    source += "put(1.0 ** 9007199254740992.0)\nput((-1.0) ** 9007199254740991.0)\nput(1.0 ** 1000000000000000000000000000000.0)\nput(math::sqrt(410))\nput(math::floor(-9007199254740991.0))\n}";
    expected += &format!("1\n-1\n1\n{}\n-9007199254740991\n", 410.0f64.sqrt());
    let mut interpreted = Vec::new();
    hibana::run(&source, &mut interpreted).unwrap();
    assert_eq!(String::from_utf8(interpreted).unwrap(), expected);
    let dir = std::env::temp_dir().join(format!("hibana-numeric-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let web = wasm_web::build(&source, &["main".into()]).unwrap();
    std::fs::write(dir.join("numeric.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let script = dir.join("run.mjs");
    std::fs::write(&script, "import {readFileSync} from 'node:fs'; import {instantiate} from './glue.mjs'; const api = await instantiate(readFileSync(new URL('./numeric.wasm', import.meta.url))); api.main();").unwrap();
    let mut commands = vec![
        { let mut c = Command::new("node"); c.arg(script); c },
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
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wasm_num_boundary_checks_arguments_and_host_results() {
    let source = "use std::math\nextern 'host' { fn twice(x: number) -> number }\nfn identity(x: number) -> number { x }\nfn calculate(x: number) -> number { twice(x) + 0.25 }\nfn floor(x: number) -> int { math::floor(x) }\nfn touch() { twice(1.0) }";
    let dir = std::env::temp_dir().join(format!("hibana-numeric-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let web = wasm_web::build(source, &["identity".into(), "calculate".into(), "floor".into()]).unwrap();
    std::fs::write(dir.join("api.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let only_import = wasm_web::build(source, &["touch".into()]).unwrap();
    std::fs::write(dir.join("touch.wasm"), only_import.wasm).unwrap();
    std::fs::write(dir.join("touch.mjs"), only_import.javascript).unwrap();
    let script = dir.join("check.mjs");
    std::fs::write(&script, r#"
import {readFileSync} from 'node:fs';
import {instantiate} from './glue.mjs';
import {instantiate as touchModule} from './touch.mjs';
const bytes = readFileSync(new URL('./api.wasm', import.meta.url));
let bad = false;
const api = await instantiate(bytes, {hibana_host: {twice: n => bad ? Infinity : n * 2}});
if (api.calculate(1.5) !== 3.25 || api.floor(-2.5) !== -3 || !Object.is(api.identity(-0), -0)) throw Error('結果が違う');
for (const value of [Infinity, -Infinity, NaN, '1', 1n]) {
  let rejected = false; try { api.identity(value); } catch { rejected = true; }
  if (!rejected) throw Error('不正な number を受け入れた');
}
bad = true;
let rejected = false; try { api.calculate(1); } catch { rejected = true; }
if (!rejected) throw Error('外部関数の結果を受け入れた');
const touch = await touchModule(readFileSync(new URL('./touch.wasm', import.meta.url)), {hibana_host: {twice: n => Infinity}});
rejected = false; try { touch.touch(); } catch { rejected = true; }
if (!rejected) throw Error('捨てる戻り値の有限性を検査していない');
const raw = new WebAssembly.Instance(new WebAssembly.Module(bytes), {hibana_host: {twice: n => n * 2}}).exports;
for (const value of [Infinity, -Infinity, NaN]) {
  let trapped = false; try { raw.identity(value); } catch { trapped = true; }
  if (!trapped) throw Error('WASM の境界で有限性を検査していない');
}
"#).unwrap();
    let result = Command::new("node").arg(script).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn javascript_checks_numeric_host_results() {
    for (ty, argument) in [("number", "1.5"), ("int", "3")] {
        let source = format!("extern '' {{ fn echo(value: {ty}) -> {ty} }}\nfn main() {{ put(echo({argument})) }}");
        let javascript = hibana::to_js(&source).unwrap();
        let mut results = vec![("value", true), ("Infinity", false), ("NaN", false), ("'wrong'", false), ("1n", false)];
        if ty == "int" { results.extend([("1.5", false), ("9007199254740992", false)]); }
        for (result, valid) in results {
            let output = Command::new("node")
                .args(["-e", &format!("globalThis.echo = value => {{ if (typeof value !== 'number') throw Error('引数の型'); return {result}; }};\n{javascript}")])
                .output().unwrap();
            assert_eq!(output.status.success(), valid, "{ty}: {}", String::from_utf8_lossy(&output.stderr));
            if valid { assert_eq!(output.stdout, format!("{argument}\n").as_bytes()); }
            else { assert!(String::from_utf8_lossy(&output.stderr).contains(ty)); }
        }
    }
}


#[test]
fn numeric_type_and_proof_boundaries() {
    for source in [
        "fn main() { let x: num = 1 }",
        "fn main() { put(9007199254740992) }",
        "fn main() { put(-9007199254740992) }",
        "fn main() { let x: int = 1.0 }",
        "fn f(x: Ref<number>) {} fn main() { let r = new_ref(1)\n f(r) }",
        "fn f(x: number) -> number[r -> r > x] { x + 1.0 }",
        "fn f(x: number[x -> x > 0.0], y: number[y -> y > 0.0]) -> number[r -> r > 0.0] { x / y }",
        "fn f(x: number[x -> x > 0.0], y: number[y -> y > 0.0]) -> number[r -> r > 0.0] { x * y }",
        "fn f(x: number[x -> 9007199254740992.0 <= x && x <= 9007199254740992.0]) -> number[r -> r > x] { x + 1.0 }",
    ] {
        let checked = if source.contains("fn main()") { source.to_string() } else { format!("{source}\nfn main() {{}}") };
        let error = hibana::to_js(&checked).unwrap_err();
        if source.contains("-> number[r") { assert!(error.to_string().contains("証明できない"), "{source}: {error}"); }
    }
    for source in [
        "fn f(x: int) -> int { x + 1 } fn main() { put(f(1)) }",
        "fn main() { put([[1], []])\n put([[], [1.5]]) }",
        "use std -> *\nfn main() { put(max(1.5, 2))\n put(max(1, 2)) }",
        "fn f(x: bool) { let v = if x { 1 } else { 2.5 }\n put(v / 2) } fn main() { f(true) }",
        "use std -> *\nfn main() { let v = match Some(1) { Some(_) -> 1 None -> 2.5 }\n put(v / 2) }",
    ] {
        assert!(hibana::to_js(source).is_ok(), "{source}");
    }
}


#[test]
fn safe_integer_traps_match_all_backends() {
    let dir = std::env::temp_dir().join(format!("hibana-safe-int-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for expression in [
        "9007199254740991 + 1", "-9007199254740991 - 1",
        "4503599627370496 * 2", "2 ** 53", "(-2) ** 53",
        "3 ** 34", "2 ** 9007199254740991",
    ] {
        let source = format!("fn main() {{ put(1)\n put({expression}) }}");
        let mut interpreted = Vec::new();
        assert!(hibana::run(&source, &mut interpreted).unwrap_err().to_string().contains("オーバーフロー"), "{expression}");
        assert_eq!(interpreted, b"1\n");
        let js = hibana::to_js(&source).unwrap();
        assert!(!js.contains("BigInt") && !js.contains("bigint"));
        let web = wasm_web::build(&source, &["main".into()]).unwrap();
        std::fs::write(dir.join("error.wasm"), web.wasm).unwrap();
        std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
        let runner = dir.join("error.mjs");
        std::fs::write(&runner, "import {readFileSync} from 'node:fs'; import {instantiate} from './glue.mjs'; const api = await instantiate(readFileSync(new URL('./error.wasm', import.meta.url))); api.main();").unwrap();
        let mut commands = vec![
            { let mut c = Command::new("node"); c.args(["-e", &js]); c },
            { let mut c = Command::new("node"); c.arg(runner); c },
        ];
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("error-native");
            std::fs::write(&path, hibana::to_native(&source).unwrap()).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            commands.push(Command::new(path));
        }
        for command in &mut commands {
            let output = command.output().unwrap();
            assert!(!output.status.success(), "{expression}: {command:?}");
            assert_eq!(output.stdout, b"1\n", "{expression}: {command:?}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn integer_boundaries_use_numbers_and_normalize_zero() {
    let source = "extern 'host' { fn echo(x: int) -> int }\nfn identity(x: int) -> int { x }\nfn from_host(x: int) -> int { echo(x) }";
    let web = wasm_web::build(source, &["identity".into(), "from_host".into()]).unwrap();
    let dir = std::env::temp_dir().join(format!("hibana-int-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("api.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let runner = dir.join("check.mjs");
    std::fs::write(&runner, r#"
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {instantiate} from './glue.mjs';
const bytes = readFileSync(new URL('./api.wasm', import.meta.url));
let result;
const api = await instantiate(bytes, {hibana_host: {echo: n => { assert.equal(typeof n, 'number'); return result; }}});
const raw = new WebAssembly.Instance(new WebAssembly.Module(bytes), {hibana_host: {echo: n => result}}).exports;
for (const n of [0, -0, 3, -3, Number.MAX_SAFE_INTEGER, -Number.MAX_SAFE_INTEGER]) {
  result = n;
  const normalized = n === 0 ? 0 : n;
  assert.equal(api.identity(n), normalized);
  assert.equal(raw.identity(n), normalized);
  assert.equal(api.from_host(1), normalized);
  assert.equal(raw.from_host(1), normalized);
}
for (const n of [1.5, -1.5, 2 ** 53, -(2 ** 53), Infinity, -Infinity, NaN, 1n, '1']) {
  assert.throws(() => api.identity(n));
  result = n;
  assert.throws(() => api.from_host(1));
}
for (const n of [1.5, -1.5, 2 ** 53, -(2 ** 53), Infinity, -Infinity, NaN]) {
  assert.throws(() => raw.identity(n), WebAssembly.RuntimeError);
  result = n;
  assert.throws(() => raw.from_host(1), WebAssembly.RuntimeError);
}
"#).unwrap();
    let output = Command::new("node").arg(runner).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(dir).unwrap();
}
