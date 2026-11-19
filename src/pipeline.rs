//! ビルドの段階。ファイルの書き込みや外部ツールの起動は呼び出し側が行う。
use crate::{error::R, ir, js, parser, typeck, wasm};
use ir::{BoundaryType, ValueType};
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

/// WASM の受け渡し規約を JSON で返す。関数の並びは指定された公開順と宣言された取り込み順。
pub fn wasm_abi(program: &ir::Program, exports: &[String]) -> R<String> {
    let exports = select_exports(program, exports)?;
    let exports = exports.iter()
        .map(|f| boundary_signature(program, &f.name, &f.param_types, f.result_type))
        .collect::<R<Vec<_>>>()?;
    let imports = program.imports.iter()
        .map(|f| boundary_signature(program, &f.name, &f.param_types, f.result_type))
        .collect::<R<Vec<_>>>()?;
    Ok(serde_json::json!({
        "exports": exports,
        "imports": imports,
        "failure_offset": wasm::FAIL_AT,
    }).to_string())
}

type BoundarySignature<'a> = (&'a str, Vec<serde_json::Value>, serde_json::Value);

fn boundary_signature<'a>(
    program: &ir::Program,
    name: &'a str,
    params: &[ValueType],
    result: ValueType,
) -> R<BoundarySignature<'a>> {
    if let Some((params, result)) = program.boundary.get(name) {
        return Ok((name, params.iter().map(BoundaryType::json).collect(), result.json()));
    }
    // 公開関数を後で選ぶ場合、基本型は関数の IR から取得できる。
    let primitive = |ty: ValueType| match ty {
        ValueType::Value | ValueType::Other => {
            Err(format!("IR は関数 {name} の境界の型に未対応").into())
        }
        ty => Ok(serde_json::Value::String(ty.to_string())),
    };
    Ok((name, params.iter().copied().map(primitive).collect::<R<_>>()?, primitive(result)?))
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
        let function = program
            .functions
            .iter()
            .find(|f| f.name == *name)
            .ok_or_else(|| format!("IR に公開する関数 {name} が無い"))?;
        if let Some(error) = function.export_error.as_deref() {
            return Err(error.clone());
        }
        if !function.boundary() {
            return Err(format!("IR は関数 {name} の境界の型に未対応").into());
        }
        selected.push(function);
    }
    Ok(selected)
}

/// 型検査済みプログラムから JavaScript を作る。
pub fn emit_js(checked: &Checked) -> R<String> {
    typeck::reject_script_calls(&checked.program, None)?;
    Ok(js::emit(&checked.program, &checked.info))
}
