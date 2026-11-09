// 型検査後の構文木から、評価順を明示した整数の中間表現を作る。
// ponytail: 整数・不変変数・関数呼び出しだけ対応する。制御フローや他の値は対応時に命令を足す。
use crate::error::R;
use crate::parser::{self, Expr, Stmt};
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write};

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub(crate) functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub(crate) name: String,
    pub(crate) params: usize,
    pub(crate) instructions: Vec<Instruction>,
    pub(crate) result: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    Int(i64),
    // 演算は 64 ビット整数で、範囲外と 0 による除算は実行時エラー。
    Binary(&'static str, usize, usize),
    Call(String, Vec<usize>),
    Print(usize),
}

impl Program {
    pub fn functions(&self) -> &[Function] {
        &self.functions
    }
}

impl Function {
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

fn is_int(e: &Expr) -> bool {
    matches!(e.peel(), Expr::Var(n) if n == "int")
}

pub fn lower(p: &parser::Program) -> R<Program> {
    lower_selected(p, None)
}

pub fn lower_selected(p: &parser::Program, exports: Option<&[String]>) -> R<Program> {
    let mut signatures = HashMap::new();
    let mut sources = HashMap::new();
    for (i, f) in p.fns.iter().enumerate() {
        signatures.insert(f.name.as_str(), (f.params.len(), f.ret.is_some()));
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
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        let f = sources[name].1;
        if f.js.is_some()
            || f.alias
            || f.op.is_some()
            || f.implicit != 0
            || f.insts != 0
            || !f.effects.is_empty()
            || !f.params.iter().all(|(_, ty)| is_int(ty))
            || f.ret.as_ref().is_some_and(|ty| !is_int(ty))
        {
            return Err(format!("IR は関数 {} の宣言に未対応", f.name).into());
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
                } if ty.as_ref().is_none_or(is_int) => {
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
        if result.is_some() != f.ret.is_some() {
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
            instructions,
            result,
        });
    }
    functions.sort_by_key(|f| sources[f.name.as_str()].0);
    Ok(Program { functions })
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
        Expr::Call(name, args) if signatures.get(name.as_str()) == Some(&(args.len(), true)) => {
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
                write!(f, "%{i}: int")?;
            }
            writeln!(
                f,
                "){} {{",
                if fun.result.is_some() { " -> int" } else { "" }
            )?;
            for (i, inst) in fun.instructions.iter().enumerate() {
                if let Instruction::Print(value) = inst {
                    writeln!(f, "  print %{value}")?;
                    continue;
                }
                write!(f, "  %{} = ", fun.params + i)?;
                match inst {
                    Instruction::Int(n) => write!(f, "int {n}")?,
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
        assert!(
            crate::to_ir("fn main() { let x = true }")
                .unwrap_err()
                .contains("未対応")
        );
        assert!(crate::to_ir("fn main() { let x = 1 + true }").is_err());
    }
}
