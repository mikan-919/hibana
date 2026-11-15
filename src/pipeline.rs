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

/// use したファイルを `base_dir` から読み、構文解析に渡すソースを作る。
/// パッケージ依存は事前に hibana fetch などで取得する。
pub fn load(source: &str, base_dir: &Path) -> R<String> {
    parser::with_base_dir(base_dir, || parser::with_uses(source))
}

/// 読み込み済みソースを構文解析し、マクロを展開して名前を解決する。
pub fn parse(source: &str) -> R<parser::Program> {
    parser::parse(source)
}

/// 明示したソースをコンパイルする。
pub fn compile(source: &str, target: &str, exports: &[String]) -> R<Vec<u8>> {
    match target {
        "wasm" => {
            let program = compiler_input(source, Kind::Library)?;
            let ir = exported_ir(&program, exports)?;
            emit_wasm(&ir, Some(exports))
        }
        "js" => {
            if !exports.is_empty() {
                return Err("js の compile は公開関数の指定に未対応。main と空の公開関数一覧を使ってください".into());
            }
            let program = compiler_input(source, Kind::Executable)?;
            let text = emit_js(&program)?;
            Ok(text.into_bytes())
        }
        "native" => {
            if !exports.is_empty() { return Err("native の公開関数一覧は空にしてください".into()); }
            let program = compiler_input(source, Kind::Executable)?;
            let roots = ["main".to_string()];
            crate::native::emit(&lower(&program, Some(&roots))?)
        }
        _ => Err(format!("未対応のコンパイル先: {target}").into()),
    }
}

/// WASM 境界で扱える公開関数と外部関数の型を JSON で返す。
/// 現在の境界型は int/str/unit。プログラム全体の型リフレクションではない。
pub fn inspect_exports(source: &str, exports: &[String]) -> R<String> {
    let program = compiler_input(source, Kind::Library)?;
    let ir = exported_ir(&program, exports)?;
    let signatures = boundary_signatures(&ir, exports)?;
    Ok(
        serde_json::json!({"exports": signatures.exports, "imports": signatures.imports})
            .to_string(),
    )
}

fn compiler_input(source: &str, kind: Kind) -> R<Checked> {
    let source = parser::with_uses(source)?;
    check(parse(&source)?, kind)
}

fn exported_ir(checked: &Checked, exports: &[String]) -> R<ir::Program> {
    if exports.is_empty() {
        return Err("WASM の公開関数が必要".into());
    }
    link(&lower(checked, Some(exports))?, exports)
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
    ir::lower_selected(&checked.program, exports)
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
        ts.contains(&ir::ValueType::Str) || r == ir::ValueType::Str
    };
    let mut visited = HashSet::new();
    let mut strings = selected
        .iter()
        .any(|f| has_str(f.param_types(), f.result_type()));
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
/// int は BigInt、str は文字列、unit は undefined として渡す。
pub fn emit_js_glue(program: &ir::Program, exports: &[String]) -> R<String> {
    let signatures = boundary_signatures(program, exports)?;
    let metadata = serde_json::json!({"exports": signatures.exports, "imports": signatures.imports}).to_string();
    js_glue(&metadata)
}

/// 境界の型情報から JavaScript 接続コードを作る。
pub fn js_glue(metadata: &str) -> R<String> {
    let metadata: serde_json::Value = serde_json::from_str(metadata).map_err(|e| format!("接続コードの型情報: {e}"))?;
    let hosts = serde_json::to_string(metadata.get("imports").ok_or("imports が必要")?).unwrap();
    let names = serde_json::to_string(metadata.get("exports").ok_or("exports が必要")?).unwrap();
    let template = r#"// hibana が生成した JavaScript 接続コード
export async function instantiate(bytes, imports = {}) {
  let instance;
  let active = false;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  function encode(value, type) {
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
    if (typeof value !== "number" && typeof value !== "bigint") throw new TypeError("整数が必要");
    if (typeof value === "number" && !Number.isSafeInteger(value)) throw new RangeError("安全な整数が必要");
    const n = BigInt(value);
    if (n < -(1n << 63n) || n >= (1n << 63n)) throw new RangeError("64 ビット整数の範囲外");
    return n;
  }
  function decode(value, type) {
    if (type === "unit") return undefined;
    if (type === "int") return value;
    const packed = BigInt.asUintN(64, value);
    const ptr = Number(packed & 0xffffffffn);
    const len = Number(packed >> 32n);
    return decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len));
  }
  const host = Object.create(null);
  for (const [name, params, result] of __HOSTS__) {
    const callback = imports.hibana_host?.[name];
    if (typeof callback !== "function") throw new TypeError("外部関数が必要: " + name);
    host[name] = (...args) => encode(callback(...args.map((value, i) => decode(value, params[i]))), result);
  }
  // put の 1 行と現在時刻。1 行は imports.hibana.print（無ければ console.log）に渡す
  const print = imports.hibana?.print ?? console.log;
  const hibana = {
    write: (fd, ptr, len) => print(decoder.decode(new Uint8Array(instance.exports.memory.buffer, Number(ptr), Number(len))).replace(/\n$/, "")),
    now_ms: () => BigInt(Date.now()),
  };
  const instantiated = await WebAssembly.instantiate(bytes, { ...imports, hibana_host: host, hibana });
  instance = instantiated instanceof WebAssembly.Instance ? instantiated : instantiated.instance;
  const functions = Object.create(null);
  for (const [name, params, result] of __EXPORTS__) {
    functions[name] = (...args) => {
      if (active) throw new Error("同じ WASM インスタンスへ再入できない");
      if (args.length !== params.length) throw new TypeError("引数の数が違う");
      active = true;
      try {
        instance.exports.__hibana_reset?.();
        const value = instance.exports[name](...args.map((value, i) => encode(value, params[i])));
        return decode(value, result);
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
    let (middle, after) = rest.split_once("__EXPORTS__").unwrap();
    Ok(format!("{before}{hosts}{middle}{names}{after}"))
}

// JSON と JS 接続コードで、境界の型名とシグネチャの並びを共有する。
type BoundarySignature<'a> = (&'a str, Vec<&'static str>, &'static str);

struct BoundarySignatures<'a> {
    exports: Vec<BoundarySignature<'a>>,
    imports: Vec<BoundarySignature<'a>>,
}

fn boundary_signature<'a>(
    name: &'a str,
    params: &[ir::ValueType],
    result: &ir::ValueType,
) -> BoundarySignature<'a> {
    fn ty(t: &ir::ValueType) -> &'static str {
        match t {
            ir::ValueType::Int => "int",
            ir::ValueType::Str => "String",
            ir::ValueType::Unit => "unit",
            ir::ValueType::Bool => "bool",
            ir::ValueType::Other => "other",
        }
    }
    (name, params.iter().map(ty).collect(), ty(result))
}

fn boundary_signatures<'a>(
    program: &'a ir::Program,
    exports: &[String],
) -> R<BoundarySignatures<'a>> {
    let exports = select_exports(program, exports)?
        .into_iter()
        .map(|f| boundary_signature(&f.name, &f.param_types, &f.result_type))
        .collect();
    let imports = program
        .imports
        .iter()
        .map(|f| boundary_signature(&f.name, &f.param_types, &f.result_type))
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
    typeck::reject_script_calls(&checked.program, &["main".into()])?;
    Ok(js::emit(&checked.program, &checked.info))
}
