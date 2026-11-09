//! ビルドの段階。ファイルの書き込みや外部ツールの起動は呼び出し側が行う。
use crate::{c, error::R, ir, js, parser, typeck, wasm};
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

/// 構文木を型検査し、必要な引数を補う。
pub fn check(mut program: parser::Program, kind: Kind) -> R<Checked> {
    let info = match kind {
        Kind::Executable => typeck::elaborate(&mut program)?,
        Kind::Library => typeck::elaborate_library(&mut program)?,
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
    let mut pending: Vec<&str> = select_exports(program, exports)?
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    let mut visited = HashSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        let function = functions
            .get(name)
            .ok_or_else(|| format!("IR に呼び出し先 {name} が無い"))?;
        for inst in &function.instructions {
            if let ir::Instruction::Call(name, _) = inst {
                pending.push(name);
            }
        }
    }
    Ok(ir::Program {
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
/// 入力の数値は安全な整数か BigInt とし、WASM には BigInt を渡す。
pub fn emit_js_glue(program: &ir::Program, exports: &[String]) -> R<String> {
    let signatures: Vec<_> = select_exports(program, exports)?
        .into_iter()
        .map(|f| (&f.name, f.params))
        .collect();
    let names =
        serde_json::to_string(&signatures).expect("関数の名前と引数の数を JSON に変換できる");
    Ok(format!(
        r#"// hibana が生成した JavaScript 接続コード
export async function instantiate(bytes, imports = {{}}) {{
  const {{ instance }} = await WebAssembly.instantiate(bytes, imports);
  const functions = Object.create(null);
  for (const [name, count] of {names}) {{
    functions[name] = (...args) => {{
      if (args.length !== count) throw new TypeError("引数の数が違う");
      return instance.exports[name](...args.map(value => {{
        if (typeof value !== "number" && typeof value !== "bigint") throw new TypeError("整数が必要");
        if (typeof value === "number" && !Number.isSafeInteger(value)) throw new RangeError("安全な整数が必要");
        const n = BigInt(value);
        if (n < -(1n << 63n) || n >= (1n << 63n)) throw new RangeError("64 ビット整数の範囲外");
        return n;
      }}));
    }};
  }}
  return functions;
}}
"#
    ))
}

fn select_exports<'a>(program: &'a ir::Program, names: &[String]) -> R<Vec<&'a ir::Function>> {
    let mut seen = HashSet::new();
    let mut selected = Vec::new();
    for name in names {
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
pub fn emit_js(checked: &Checked) -> String {
    js::emit(&checked.program, &checked.info)
}

/// 型検査済みプログラムから C を作る。
pub fn emit_c(checked: &Checked) -> R<String> {
    if let Some(f) = checked
        .program
        .fns
        .iter()
        .find(|f| f.js.as_ref().is_some_and(|m| !m.is_empty()))
    {
        return Err(format!(
            "extern の関数 {} は JS ターゲット（hibana js）でだけ使える",
            f.name
        )
        .into());
    }
    Ok(c::emit(&checked.program))
}
