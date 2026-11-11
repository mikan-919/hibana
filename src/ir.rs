// 型検査後の構文木から、評価順を明示した中間表現を作る。
// ponytail: 整数・文字列・不変変数・関数呼び出しに対応する。制御フローや永続オブジェクトは未対応。
use crate::error::R;
use crate::parser::{self, Expr, Stmt};
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write};

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub(crate) functions: Vec<Function>,
    pub(crate) imports: Vec<Import>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Int,
    Str,
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub(crate) name: String,
    pub(crate) param_types: Vec<ValueType>,
    pub(crate) result_type: ValueType,
}
impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Int => "int",
            Self::Str => "str",
            Self::Unit => "unit",
        })
    }
}
impl Import {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn param_types(&self) -> &[ValueType] {
        &self.param_types
    }
    pub fn result_type(&self) -> ValueType {
        self.result_type
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub(crate) name: String,
    pub(crate) params: usize,
    pub(crate) param_types: Vec<ValueType>,
    pub(crate) result_type: ValueType,
    pub(crate) instructions: Vec<Instruction>,
    pub(crate) result: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    Int(i64),
    Str(String),
    // 演算は 64 ビット整数で、範囲外と 0 による除算は実行時エラー。
    Binary(&'static str, usize, usize),
    Call(String, Vec<usize>),
    Print(usize),
}

impl Program {
    pub fn imports(&self) -> &[Import] {
        &self.imports
    }
    pub fn functions(&self) -> &[Function] {
        &self.functions
    }
}

impl Function {
    pub fn param_types(&self) -> &[ValueType] {
        &self.param_types
    }
    pub fn result_type(&self) -> ValueType {
        self.result_type
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn params(&self) -> usize {
        self.params
    }
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }
    pub fn result(&self) -> Option<usize> {
        self.result
    }
}

fn value_type(e: &Expr) -> Option<ValueType> {
    match e.peel() {
        Expr::Var(n) if n == "int" => Some(ValueType::Int),
        Expr::Var(n) if n == "str" => Some(ValueType::Str),
        Expr::Var(n) if n == "unit" => Some(ValueType::Unit),
        _ => None,
    }
}
fn supported(e: &Expr) -> bool {
    value_type(e).is_some()
}

pub fn lower(p: &parser::Program) -> R<Program> {
    lower_selected(p, None)
}

pub fn lower_selected(p: &parser::Program, exports: Option<&[String]>) -> R<Program> {
    let mut signatures = HashMap::new();
    let mut sources = HashMap::new();
    for (i, f) in p.fns.iter().enumerate() {
        signatures.insert(
            f.name.as_str(),
            (
                f.params.len(),
                f.ret
                    .as_ref()
                    .is_some_and(|t| value_type(t) != Some(ValueType::Unit)),
            ),
        );
        sources.insert(f.name.as_str(), (i, f));
    }
    if exports.is_none() && (!p.datas.is_empty() || !p.tests.is_empty()) {
        return Err("IR は型宣言とテスト宣言に未対応".into());
    }
    let mut pending = match exports {
        Some(names) => {
            let mut seen = HashSet::new();
            for name in names {
                if !signatures.contains_key(name.as_str()) {
                    return Err(format!("公開する関数 {name} が無い").into());
                }
                if matches!(
                    name.as_str(),
                    "memory" | "__hibana_alloc" | "__hibana_reset"
                ) {
                    return Err(format!("公開名 {name} は WASM ABI の予約名").into());
                }
                if !seen.insert(name) {
                    return Err(format!("公開する関数 {name} が重複している").into());
                }
            }
            names.iter().map(String::as_str).collect::<Vec<_>>()
        }
        None => p.fns.iter().map(|f| f.name.as_str()).collect(),
    };
    let mut visited = HashSet::new();
    let mut functions = Vec::new();
    let mut imports = Vec::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        let f = sources[name].1;
        if f.alias
            || f.op.is_some()
            || f.implicit != 0
            || f.insts != 0
            || !f.effects.is_empty()
            || !f
                .params
                .iter()
                .all(|(_, ty)| matches!(value_type(ty), Some(ValueType::Int | ValueType::Str)))
            || f.ret.as_ref().is_some_and(|ty| !supported(ty))
        {
            return Err(format!("IR は関数 {} の宣言に未対応", f.name).into());
        }
        let param_types = f
            .params
            .iter()
            .map(|(_, t)| value_type(t).unwrap())
            .collect::<Vec<_>>();
        let result_type = f
            .ret
            .as_ref()
            .and_then(value_type)
            .unwrap_or(ValueType::Unit);
        if f.js.is_some() {
            if exports.is_some_and(|names| names.contains(&f.name)) {
                return Err("外部関数を直接公開できない".into());
            }
            imports.push(Import {
                name: f.name.clone(),
                param_types,
                result_type,
            });
            continue;
        }
        let mut vars = HashMap::new();
        for (i, (name, _)) in f.params.iter().enumerate() {
            vars.insert(name.as_str(), i);
        }
        let mut instructions = Vec::new();
        for stmt in &f.body.stmts {
            match stmt {
                Stmt::Let {
                    name,
                    mutable: false,
                    ty,
                    init,
                } if ty.as_ref().is_none_or(supported) => {
                    let value = expr(init, &vars, &mut instructions, f.params.len(), &signatures)?;
                    vars.insert(name, value);
                }
                Stmt::Expr(e) if matches!(e.peel(), Expr::Call(name, args) if name == "print" && args.len() == 1) =>
                {
                    let Expr::Call(_, args) = e.peel() else {
                        unreachable!()
                    };
                    let value = expr(
                        &args[0],
                        &vars,
                        &mut instructions,
                        f.params.len(),
                        &signatures,
                    )?;
                    check_print(value, &instructions, &param_types, &sources)?;
                    instructions.push(Instruction::Print(value));
                }
                Stmt::Expr(e) => {
                    expr(e, &vars, &mut instructions, f.params.len(), &signatures)?;
                }
                _ => return Err("IR はこの文に未対応".into()),
            }
        }
        let result = match f.body.tail.as_deref().map(Expr::peel) {
            Some(Expr::Call(name, args)) if name == "print" && args.len() == 1 => {
                let value = expr(
                    &args[0],
                    &vars,
                    &mut instructions,
                    f.params.len(),
                    &signatures,
                )?;
                check_print(value, &instructions, &param_types, &sources)?;
                instructions.push(Instruction::Print(value));
                None
            }
            Some(e) => Some(expr(
                e,
                &vars,
                &mut instructions,
                f.params.len(),
                &signatures,
            )?),
            None => None,
        };
        let result = if result_type == ValueType::Unit {
            None
        } else {
            result
        };
        if result.is_some() != (result_type != ValueType::Unit) {
            return Err(format!("IR は関数 {} の戻り値に未対応", f.name).into());
        }
        for inst in &instructions {
            if let Instruction::Call(name, _) = inst {
                pending.push(sources[name.as_str()].1.name.as_str());
            }
        }
        functions.push(Function {
            name: f.name.clone(),
            params: f.params.len(),
            param_types,
            result_type,
            instructions,
            result,
        });
    }
    functions.sort_by_key(|f| sources[f.name.as_str()].0);
    imports.sort_by_key(|f| sources[f.name.as_str()].0);
    Ok(Program { functions, imports })
}

fn check_print(
    value: usize,
    instructions: &[Instruction],
    params: &[ValueType],
    sources: &HashMap<&str, (usize, &parser::Fn)>,
) -> R<()> {
    let ty = if value < params.len() {
        params[value]
    } else {
        match &instructions[value - params.len()] {
            Instruction::Int(_) | Instruction::Binary(..) => ValueType::Int,
            Instruction::Str(_) => ValueType::Str,
            Instruction::Call(name, _) => sources[name.as_str()]
                .1
                .ret
                .as_ref()
                .and_then(value_type)
                .unwrap_or(ValueType::Unit),
            Instruction::Print(_) => ValueType::Unit,
        }
    };
    if ty == ValueType::Int {
        Ok(())
    } else {
        Err("IR の print は整数以外に未対応".into())
    }
}

fn expr(
    e: &Expr,
    vars: &HashMap<&str, usize>,
    out: &mut Vec<Instruction>,
    params: usize,
    signatures: &HashMap<&str, (usize, bool)>,
) -> R<usize> {
    let inst = match e.peel() {
        Expr::Int(n) => Instruction::Int(*n),
        Expr::Str(s) => Instruction::Str(s.clone()),
        Expr::Var(n) => {
            return vars
                .get(n.as_str())
                .copied()
                .ok_or_else(|| format!("IR は変数 {n} に未対応").into());
        }
        Expr::Binary(op @ ("+" | "-" | "*" | "/" | "%"), a, b) => {
            let a = expr(a, vars, out, params, signatures)?;
            let b = expr(b, vars, out, params, signatures)?;
            Instruction::Binary(op, a, b)
        }
        Expr::Call(name, args)
            if signatures
                .get(name.as_str())
                .is_some_and(|(n, _)| *n == args.len()) =>
        {
            let mut values = Vec::new();
            for arg in args {
                values.push(expr(arg, vars, out, params, signatures)?);
            }
            Instruction::Call(name.clone(), values)
        }
        _ => return Err("IR はこの式に未対応".into()),
    };
    let value = params + out.len();
    out.push(inst);
    Ok(value)
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for fun in &self.functions {
            write!(f, "fn {}(", fun.name)?;
            for i in 0..fun.params {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "%{i}: {}", fun.param_types[i])?;
            }
            if fun.result.is_some() {
                write!(f, ") -> {}", fun.result_type)?;
            } else {
                f.write_str(")")?;
            }
            writeln!(f, " {{")?;
            for (i, inst) in fun.instructions.iter().enumerate() {
                if let Instruction::Print(value) = inst {
                    writeln!(f, "  print %{value}")?;
                    continue;
                }
                write!(f, "  %{} = ", fun.params + i)?;
                match inst {
                    Instruction::Int(n) => write!(f, "int {n}")?,
                    Instruction::Str(s) => write!(f, "str {s:?}")?,
                    Instruction::Binary(op, a, b) => write!(f, "{op} %{a}, %{b}")?,
                    Instruction::Call(name, args) => {
                        write!(f, "call {name}(")?;
                        for (i, arg) in args.iter().enumerate() {
                            if i > 0 {
                                f.write_str(", ")?;
                            }
                            write!(f, "%{arg}")?;
                        }
                        f.write_char(')')?;
                    }
                    Instruction::Print(_) => unreachable!(),
                }
                writeln!(f)?;
            }
            match fun.result {
                Some(value) => writeln!(f, "  return %{value}")?,
                None => writeln!(f, "  return")?,
            }
            writeln!(f, "}}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lowers_checked_program_in_evaluation_order() {
        let src = "fn add(x: int, y: int) -> int { x + y }\nfn main() { let x = add(1, 2)\n let x = x * 3\n x + 4; }";
        let ir = crate::to_ir(src).unwrap();
        assert_eq!(
            ir,
            "fn add(%0: int, %1: int) -> int {\n  %2 = + %0, %1\n  return %2\n}\nfn main() {\n  %0 = int 1\n  %1 = int 2\n  %2 = call add(%0, %1)\n  %3 = int 3\n  %4 = * %2, %3\n  %5 = int 4\n  %6 = + %4, %5\n  return\n}\n"
        );
    }

    #[test]
    fn rejects_unsupported_and_invalid_programs() {
        assert!(crate::to_ir("fn main() { let x = true }")
            .unwrap_err()
            .contains("未対応"));
        assert!(crate::to_ir("fn main() { let x = 1 + true }").is_err());
    }
}
