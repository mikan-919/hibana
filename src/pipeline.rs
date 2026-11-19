//! ビルドの段階。ファイルの書き込みや外部ツールの起動は呼び出し側が行う。
use crate::{error::R, ir, js, parser, typeck, wasm};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// 実行ファイルは main を要求し、ライブラリは要求しない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Executable,
    Library,
}

/// 型検査と省略された引数の補完を終えたプログラム。
pub struct Checked {
    pub(crate) program: parser::Program,
    info: typeck::Info,
}

impl Checked {
    /// 型検査後の構文木を読む。
    pub fn program(&self) -> &parser::Program {
        &self.program
    }
}

/// use したファイルを `base_dir` から読み、構文解析に渡すモジュール表を作る。
/// 組み込みライブラリと、名前で指定した hibana ファイル、JavaScript / TypeScript の型宣言を読み込む。
pub fn load(source: &str, base_dir: &Path) -> R<parser::ModuleSet> {
    parser::load_modules(source, base_dir)
}

/// 読み込み済みソースを構文解析し、マクロを展開して名前を解決する。
pub fn parse(source: impl Into<parser::ModuleSet>) -> R<parser::Program> {
    parser::parse_modules(source.into())
}

/// 構文木を型検査し、必要な引数を補う。
pub fn check(mut program: parser::Program, kind: Kind) -> R<Checked> {
    let info = match kind {
        Kind::Executable => typeck::check(&mut program)?,
        Kind::Library => typeck::check_library(&mut program)?,
    };
    Ok(Checked { program, info })
}

/// 公開する関数から辿れる関数を IR に変換する。None はすべての関数を変換する。
pub fn lower(checked: &Checked, exports: Option<&[String]>) -> R<ir::Program> {
    typeck::reject_script_calls(&checked.program, exports)?;
    ir::lower_selected(&checked.program, exports, &checked.info)
}

/// IR から公開関数とその呼び出し先を選ぶ。
pub fn link(program: &ir::Program, exports: &[String]) -> R<ir::Program> {
    let functions: HashMap<_, _> = program
        .functions
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    let selected = select_exports(program, exports)?;
    let mut pending: Vec<&str> = selected.iter().map(|f| f.name.as_str()).collect();
    let has_str = |ts: &[ir::ValueType], r: ir::ValueType| {
        ts.iter().chain([&r]).any(|t| matches!(t, ir::ValueType::Str))
    };
    let mut visited = HashSet::new();
    let mut strings = selected
        .iter()
        .any(|f| has_str(f.param_types(), f.result_type()));
    let has_value = |ts: &[ir::ValueType], r: ir::ValueType| ts.iter().chain([&r]).any(|t| *t == ir::ValueType::Value);
    if selected.iter().any(|f| has_value(f.param_types(), f.result_type())) {
        pending.extend(["__val_in", "__val_out"]);
    }
    // 境界で文字列を渡すなら、変換するランタイムの関数も含める
    loop {
        if strings {
            pending.extend(["__str_in", "__str_out"]);
        }
        while let Some(name) = pending.pop() {
            if !visited.insert(name) {
                continue;
            }
            if let Some(f) = program.imports.iter().find(|f| f.name() == name) {
                strings |= has_str(f.param_types(), f.result_type());
                if has_value(f.param_types(), f.result_type()) { pending.extend(["__val_in", "__val_out"]); }
                if f.param_types().contains(&ir::ValueType::Int) || f.result_type() == ir::ValueType::Int { pending.push("__int_in"); }
                if f.param_types().contains(&ir::ValueType::Number) || f.result_type() == ir::ValueType::Number { pending.push("__num_checked"); }
                continue;
            }
            let function = functions
                .get(name)
                .ok_or_else(|| format!("IR に呼び出し先 {name} が無い"))?;
            pending.extend(function.calls());
        }
        if !strings || visited.contains("__str_in") {
            break;
        }
    }
    Ok(ir::Program {
        imports: program
            .imports
            .iter()
            .filter(|f| visited.contains(f.name.as_str()))
            .cloned()
            .collect(),
        functions: program
            .functions
            .iter()
            .filter(|f| visited.contains(f.name.as_str()))
            .cloned()
            .collect(),
        boundary: program.boundary.clone(),
    })
}

/// IR を WASM に変換し、指定した関数を公開する。None はすべての関数を公開する。
pub fn emit_wasm(program: &ir::Program, exports: Option<&[String]>) -> R<Vec<u8>> {
    if let Some(names) = exports {
        select_exports(program, names)?;
    }
    Ok(wasm::emit_selected(program, exports))
}

/// 公開関数を JavaScript から呼ぶための ES モジュールを作る。
/// int と number は Number、str は文字列、unit は undefined として渡す。
pub fn emit_js_glue(program: &ir::Program, exports: &[String]) -> R<String> {
    let signatures = boundary_signatures(program, exports)?;
    let hosts = serde_json::to_string(&signatures.imports).unwrap();
    let names = serde_json::to_string(&signatures.exports).unwrap();
    let template = r#"// hibana が生成した JavaScript 接続コード
export async function instantiate(bytes, imports = {}) {
  let instance;
  let active = false;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
  // Vec と構造体は 8 バイトの語の並びで渡す（runtime.hb の __val_in・__val_out）。
  // 値は [型の番号][中身]、文字列は [3][長さ][バイト列]、Vec は [4][要素数][要素...]、構造体は [5][構築子][フィールド数][フィールド...]
  function put(out, type, value) {
    if (typeof type === "object" && type.vec !== undefined) {
      if (!Array.isArray(value)) throw new TypeError("配列が必要");
      out.push(4n, BigInt(value.length));
      for (const item of value) put(out, type.vec, item);
    } else if (typeof type === "object") {
      if (typeof value !== "object" || value === null) throw new TypeError("オブジェクトが必要");
      out.push(5n, BigInt(type.ctor), BigInt(type.fields.length));
      for (const [name, field] of type.fields) {
        if (!(name in value)) throw new TypeError("フィールドが必要: " + name);
        put(out, field, value[name]);
      }
    } else if (type === "String") {
      if (typeof value !== "string") throw new TypeError("文字列が必要");
      const data = encoder.encode(value);
      out.push(3n, BigInt(data.length));
      for (let i = 0; i < data.length; i += 8) {
        let word = 0n;
        for (let k = 7; k >= 0; k--) word = (word << 8n) | BigInt(data[i + k] ?? 0);
        out.push(BigInt.asIntN(64, word));
      }
    } else if (type === "number") {
      if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError("有限な number が必要");
      out.push(9n, new BigInt64Array(new Float64Array([value]).buffer)[0]);
    } else if (type === "bool") {
      if (typeof value !== "boolean") throw new TypeError("真偽値が必要");
      out.push(2n, value ? 1n : 0n);
    } else if (type === "unit") {
      if (value !== undefined) throw new TypeError("unit は undefined が必要");
      out.push(0n);
    } else {
      if (typeof value !== "number" || !Number.isSafeInteger(value)) throw new RangeError("安全な整数が必要");
      out.push(1n, BigInt(value));
    }
  }
  function take(view, at, type) {
    const word = (k) => view.getBigInt64(at + 8 * k, true);
    if (typeof type === "object" && type.vec !== undefined) {
      const items = [];
      let next = at + 16;
      for (let i = 0; i < Number(word(1)); i++) {
        const [item, after] = take(view, next, type.vec);
        items.push(item);
        next = after;
      }
      return [items, next];
    }
    if (typeof type === "object") {
      const object = {};
      let next = at + 24;
      for (const [name, field] of type.fields) {
        const [item, after] = take(view, next, field);
        object[name] = item;
        next = after;
      }
      return [object, next];
    }
    if (type === "String") {
      const n = Number(word(1));
      return [decoder.decode(new Uint8Array(view.buffer, view.byteOffset + at + 16, n)), at + 16 + Math.ceil(n / 8) * 8];
    }
    if (type === "unit") return [undefined, at + 8];
    if (type === "bool") return [word(1) !== 0n, at + 16];
    if (word(0) === 9n) return [view.getFloat64(at + 8, true), at + 16];
    const n = Number(word(1));
    return [n === 0 ? 0 : n, at + 16];
  }
  function encode(value, type) {
    if (typeof type === "object") {
      const words = [];
      put(words, type, value);
      const data = new Uint8Array(new BigInt64Array(words).buffer);
      const ptr = instance.exports.__hibana_alloc(data.length) >>> 0;
      new Uint8Array(instance.exports.memory.buffer, ptr, data.length).set(data);
      return BigInt.asIntN(64, (BigInt(data.length) << 32n) | BigInt(ptr));
    }
    if (type === 'bool') {
      if (typeof value !== 'boolean') throw new TypeError('真偽値が必要');
      return value ? 1n : 0n;
    }
    if (type === "unit") {
      if (value !== undefined) throw new TypeError("unit の戻り値は undefined が必要");
      return undefined;
    }
    if (type === "String") {
      if (typeof value !== "string") throw new TypeError("文字列が必要");
      const data = encoder.encode(value);
      const ptr = instance.exports.__hibana_alloc(data.length) >>> 0;
      new Uint8Array(instance.exports.memory.buffer, ptr, data.length).set(data);
      return BigInt.asIntN(64, (BigInt(data.length) << 32n) | BigInt(ptr));
    }
    if (type === "number") {
      if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError("有限な number が必要");
      return value;
    }
    if (typeof value !== "number" || !Number.isSafeInteger(value)) throw new RangeError("安全な整数が必要");
    return value === 0 ? 0 : value;
  }
  function decode(value, type) {
    if (typeof type === "object") {
      const packed = BigInt.asUintN(64, value);
      const view = new DataView(instance.exports.memory.buffer, Number(packed & 0xffffffffn), Number(packed >> 32n));
      return take(view, 0, type)[0];
    }
    if (type === 'bool') return value !== 0n;
    if (type === "unit") return undefined;
    if (type === "int") {
      if (typeof value !== "number" || !Number.isSafeInteger(value)) throw new RangeError("安全な整数が必要");
      return value === 0 ? 0 : value;
    }
    if (type === "number") {
      if (!Number.isFinite(value)) throw new RangeError("number の演算結果が有限でない");
      return value;
    }
    const packed = BigInt.asUintN(64, value);
    const ptr = Number(packed & 0xffffffffn);
    const len = Number(packed >> 32n);
    return decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len));
  }
  const host = Object.create(null);
  const hostSignatures = __HOSTS__;
  for (const [name, params, result] of hostSignatures) {
    const callback = imports.hibana_host?.[name];
    if (typeof callback !== "function") throw new TypeError("外部関数が必要: " + name);
    host[name] = (...args) => encode(callback(...args.map((value, i) => decode(value, params[i]))), result);
  }
  // 標準出力と標準エラーは別の関数に渡す
  const print = imports.hibana?.print ?? console.log;
  const error = imports.hibana?.error ?? console.error;
  const hibana = {
    write: (fd, ptr, len) => {
      const output = Number(fd) === 2 ? error : print;
      output(decoder.decode(new Uint8Array(instance.exports.memory.buffer, Number(ptr), Number(len))).replace(/\n$/, ""));
    },
    now_ms: () => BigInt(Date.now()),
    read: (fd, ptr, len) => {
      const target = new Uint8Array(instance.exports.memory.buffer, Number(ptr), Number(len));
      if (imports.hibana?.read) return BigInt(imports.hibana.read(Number(fd), target));
      if (typeof process !== "undefined" && process.getBuiltinModule) {
        const fs = process.getBuiltinModule("node:fs");
        try { return BigInt(fs.readSync(Number(fd), target, 0, target.length, null)); }
        catch { return -1n; }
      }
      throw new Error("標準入力には imports.hibana.read が必要です");
    },
  };
  const instantiated = await WebAssembly.instantiate(bytes, { ...imports, hibana_host: host, hibana });
  instance = instantiated instanceof WebAssembly.Instance ? instantiated : instantiated.instance;
  const functions = Object.create(null);
  const exportSignatures = __EXPORTS__;
  for (const [name, params, result] of exportSignatures) {
    functions[name] = (...args) => {
      if (active) throw new Error("同じ WASM インスタンスへ再入できない");
      if (args.length !== params.length) throw new TypeError("引数の数が違う");
      active = true;
      try {
        instance.exports.__hibana_reset?.();
        const value = instance.exports[name](...args.map((value, i) => encode(value, params[i])));
        return decode(value, result);
      } catch (e) {
        if (e instanceof WebAssembly.RuntimeError && instance.exports.memory) {
          const memory = instance.exports.memory.buffer;
          const view = new DataView(memory);
          const failure = __FAIL_AT__;
          const ptr = Number(view.getBigUint64(failure, true));
          if (ptr !== 0) {
            view.setBigUint64(failure, 0n, true);
            const len = Number(view.getBigUint64(ptr, true));
            e.message = "実行時エラー: " + decoder.decode(new Uint8Array(memory, ptr + 8, len));
          }
        }
        throw e;
      } finally {
        instance.exports.__hibana_reset?.();
        active = false;
      }
    };
  }
  return functions;
}
"#;
    // 挿入した関数名に含まれる文字列を、後続の置換で変更しない。
    let (before, rest) = template.split_once("__HOSTS__").unwrap();
    let (middle, rest) = rest.split_once("__EXPORTS__").unwrap();
    let (exports, after) = rest.split_once("__FAIL_AT__").unwrap();
    let fail_at = wasm::FAIL_AT;
    Ok(format!("{before}{hosts}{middle}{names}{exports}{fail_at}{after}"))
}

// JS 接続コードに渡す境界の型名とシグネチャ。
type BoundarySignature<'a> = (&'a str, Vec<serde_json::Value>, serde_json::Value);

struct BoundarySignatures<'a> {
    exports: Vec<BoundarySignature<'a>>,
    imports: Vec<BoundarySignature<'a>>,
}

// 型は名前の文字列。Vec と構造体は ir::BoundaryType::json の形
fn boundary_signature<'a>(
    program: &ir::Program,
    name: &'a str,
    params: &[ir::ValueType],
    result: &ir::ValueType,
) -> BoundarySignature<'a> {
    if let Some((ps, r)) = program.boundary.get(name) {
        return (name, ps.iter().map(|t| t.json()).collect(), r.json());
    }
    let ty = |t: &ir::ValueType| serde_json::Value::String(t.to_string());
    (name, params.iter().map(ty).collect(), ty(result))
}

fn boundary_signatures<'a>(
    program: &'a ir::Program,
    exports: &[String],
) -> R<BoundarySignatures<'a>> {
    let exports = select_exports(program, exports)?
        .into_iter()
        .map(|f| boundary_signature(program, &f.name, &f.param_types, &f.result_type))
        .collect();
    let imports = program
        .imports
        .iter()
        .map(|f| boundary_signature(program, &f.name, &f.param_types, &f.result_type))
        .collect();
    Ok(BoundarySignatures { exports, imports })
}

fn select_exports<'a>(program: &'a ir::Program, names: &[String]) -> R<Vec<&'a ir::Function>> {
    let mut seen = HashSet::new();
    let mut selected = Vec::new();
    for name in names {
        if matches!(
            name.as_str(),
            "memory" | "__hibana_alloc" | "__hibana_reset"
        ) {
            return Err(format!("公開する関数名 {name} は WASM の内部用に予約されている").into());
        }
        if !seen.insert(name) {
            return Err(format!("公開する関数 {name} が重複している").into());
        }
        selected.push(
            program
                .functions
                .iter()
                .find(|f| f.name == *name)
                .ok_or_else(|| format!("IR に公開する関数 {name} が無い"))?,
        );
    }
    Ok(selected)
}

/// 型検査済みプログラムから JavaScript を作る。
pub fn emit_js(checked: &Checked) -> R<String> {
    typeck::reject_script_calls(&checked.program, None)?;
    Ok(js::emit(&checked.program, &checked.info))
}
