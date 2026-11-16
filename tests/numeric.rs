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
    let mut source = String::from("use std::math\nfn main() {\n");
    let mut expected = String::new();
    for n in numbers {
        source += &format!("put({})\n", literal(n));
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
    source += "put(math::sqrt(410))\nput(math::floor(-9223372036854775808.0))\n}";
    expected += &format!("{}\n-9223372036854775808\n", 410.0f64.sqrt());
    let mut interpreted = Vec::new();
    hibana::run(&source, &mut interpreted).unwrap();
    assert_eq!(String::from_utf8(interpreted).unwrap(), expected);
    let dir = std::env::temp_dir().join(format!("hibana-numeric-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let web = hibana::to_web(&source, &["main".into()]).unwrap();
    std::fs::write(dir.join("numeric.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let script = dir.join("run.mjs");
    std::fs::write(&script, "import {readFileSync} from 'node:fs'; import {instantiate} from './glue.mjs'; const api = await instantiate(readFileSync(new URL('./numeric.wasm', import.meta.url))); api.main();").unwrap();
    let mut commands = vec![{ let mut c = Command::new("node"); c.arg(script); c }];
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
    let source = "use std::math\nextern 'host' { fn twice(x: num) -> num }\nfn identity(x: num) -> num { x }\nfn calculate(x: num) -> num { twice(x) + 0.25 }\nfn floor(x: num) -> int { math::floor(x) }\nfn touch() { twice(1.0) }";
    let dir = std::env::temp_dir().join(format!("hibana-numeric-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let web = hibana::to_web(source, &["identity".into(), "calculate".into(), "floor".into()]).unwrap();
    std::fs::write(dir.join("api.wasm"), web.wasm).unwrap();
    std::fs::write(dir.join("glue.mjs"), web.javascript).unwrap();
    let only_import = hibana::to_web(source, &["touch".into()]).unwrap();
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
if (api.calculate(1.5) !== 3.25 || api.floor(-2.5) !== -3n || !Object.is(api.identity(-0), -0)) throw Error('結果が違う');
for (const value of [Infinity, -Infinity, NaN, '1', 1n]) {
  let rejected = false; try { api.identity(value); } catch { rejected = true; }
  if (!rejected) throw Error('不正な num を受け入れた');
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
