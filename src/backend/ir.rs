// 型検査後の構文木から、WASM とネイティブの両方へ出す構造化した中間表現を作る。
// 制御フローは WASM と同じ入れ子の形（if・while・return）で持ち、goto を持たない。
// 値はすべて 64 ビット整数で表す。bool は 0 と 1、unit は 0。文字列は出力先ごとに表し方を決める値。
// ponytail: 整数・真偽値・文字列のリテラル・if・while・範囲の for・関数呼び出し・print に対応する。ヒープ・Vec・構造体・クロージャ・効果は未対応
use crate::error::R;
use crate::parser::{self, Expr as Ast, Stmt as AstStmt};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub(crate) functions: Vec<Function>,
    pub(crate) imports: Vec<Import>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Int,
    Bool,
    Str,
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub(crate) name: String,
    pub(crate) param_types: Vec<ValueType>,
    pub(crate) result_type: ValueType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub(crate) name: String,
    pub(crate) param_types: Vec<ValueType>,
    pub(crate) result_type: ValueType,
    // 引数を含む局所変数の数。引数は 0 から順に並ぶ
    pub(crate) locals: u32,
    pub(crate) body: Vec<Stmt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    // 算術は 64 ビット整数で、範囲外と 0 による除算は実行時エラー
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    // 比較は 0 か 1 を返す
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Int(i64),
    Str(String),
    Local(u32),
    Binary(Op, Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    // unit を返す関数の呼び出しは 0 になる
    Call(String, Vec<Expr>),
    If(Box<Expr>, Box<Block>, Box<Block>),
    Block(Box<Block>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub(crate) stmts: Vec<Stmt>,
    pub(crate) value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Set(u32, Expr),
    Eval(Expr),
    // 値と改行を出力する。型は Int か Str
    Print(ValueType, Expr),
    While(Expr, Vec<Stmt>),
    Return(Option<Expr>),
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Str => "str",
            Self::Unit => "unit",
        })
    }
}

impl Program {
    pub fn imports(&self) -> &[Import] {
        &self.imports
    }
    pub fn functions(&self) -> &[Function] {
        &self.functions
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

impl Function {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn param_types(&self) -> &[ValueType] {
        &self.param_types
    }
    pub fn result_type(&self) -> ValueType {
        self.result_type
    }
    pub fn body(&self) -> &[Stmt] {
        &self.body
    }
    // 本体に現れるすべての文と式（入れ子を含む）を、出現順に f に渡す
    pub fn each<'a>(&'a self, f: &mut impl FnMut(Node<'a>)) {
        each_stmts(&self.body, f)
    }
    pub fn calls(&self) -> Vec<&str> {
        let mut names = Vec::new();
        self.each(&mut |n| {
            if let Node::Expr(Expr::Call(n, _)) = n {
                names.push(n.as_str())
            }
        });
        names
    }
}

#[derive(Clone, Copy)]
pub enum Node<'a> {
    Stmt(&'a Stmt),
    Expr(&'a Expr),
}

fn each_stmts<'a>(stmts: &'a [Stmt], f: &mut impl FnMut(Node<'a>)) {
    for s in stmts {
        f(Node::Stmt(s));
        match s {
            Stmt::Set(_, e) | Stmt::Eval(e) | Stmt::Print(_, e) | Stmt::Return(Some(e)) => {
                each_expr(e, f)
            }
            Stmt::While(c, body) => {
                each_expr(c, f);
                each_stmts(body, f);
            }
            Stmt::Return(None) => {}
        }
    }
}

fn each_expr<'a>(e: &'a Expr, f: &mut impl FnMut(Node<'a>)) {
    f(Node::Expr(e));
    match e {
        Expr::Int(_) | Expr::Str(_) | Expr::Local(_) => {}
        Expr::Binary(_, a, b) => {
            each_expr(a, f);
            each_expr(b, f);
        }
        Expr::Not(a) => each_expr(a, f),
        Expr::Call(_, args) => args.iter().for_each(|a| each_expr(a, f)),
        Expr::If(c, t, e) => {
            each_expr(c, f);
            each_block(t, f);
            each_block(e, f);
        }
        Expr::Block(b) => each_block(b, f),
    }
}

fn each_block<'a>(b: &'a Block, f: &mut impl FnMut(Node<'a>)) {
    each_stmts(&b.stmts, f);
    each_expr(&b.value, f);
}

fn value_type(e: &Ast) -> Option<ValueType> {
    match e.peel() {
        Ast::Var(n) if n == "int" => Some(ValueType::Int),
        Ast::Var(n) if n == "bool" => Some(ValueType::Bool),
        Ast::Var(n) if n == "str" => Some(ValueType::Str),
        Ast::Var(n) if n == "unit" => Some(ValueType::Unit),
        Ast::Refine(base, _, _) => value_type(base),
        _ => None,
    }
}

// 関数の引数と戻り値の型。IR で表せなければ None
type Signature = Option<(Vec<ValueType>, ValueType)>;

fn signature(f: &parser::Fn) -> Signature {
    if f.alias || f.op.is_some() || f.implicit != 0 || f.insts != 0 || !f.effects.is_empty() {
        return None;
    }
    let params = f
        .params
        .iter()
        .map(|(_, t)| value_type(t).filter(|t| *t != ValueType::Unit))
        .collect::<Option<Vec<_>>>()?;
    let result = match &f.ret {
        Some(t) => value_type(t)?,
        None => ValueType::Unit,
    };
    Some((params, result))
}

pub fn lower(p: &parser::Program) -> R<Program> {
    lower_selected(p, None)
}

/// exports から辿れる関数だけを変換する。None はすべての関数を変換する。
pub fn lower_selected(p: &parser::Program, exports: Option<&[String]>) -> R<Program> {
    let mut sources = HashMap::new();
    let mut signatures = HashMap::new();
    for (i, f) in p.fns.iter().enumerate() {
        sources.insert(f.name.as_str(), (i, f));
        signatures.insert(f.name.as_str(), signature(f));
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
    let exported = |name: &str| exports.is_some_and(|names| names.iter().any(|n| n == name));
    let mut visited = HashSet::new();
    let mut functions = Vec::new();
    let mut imports = Vec::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        let f = sources[name].1;
        let Some((param_types, result_type)) = signatures[name].clone() else {
            return Err(format!("IR は関数 {} の宣言に未対応", f.name).into());
        };
        // ponytail: WASM の境界（公開関数と外部関数）は int・str・unit だけ。bool を渡すなら接続コードに変換を足す
        if (exported(name) || f.js.is_some())
            && (param_types.contains(&ValueType::Bool) || result_type == ValueType::Bool)
        {
            return Err(format!("IR は関数 {} の境界の bool に未対応", f.name).into());
        }
        if f.js.is_some() {
            if exported(name) {
                return Err("外部関数を直接公開できない".into());
            }
            imports.push(Import {
                name: f.name.clone(),
                param_types,
                result_type,
            });
            continue;
        }
        let mut l = Lower {
            signatures: &signatures,
            vars: HashMap::new(),
            locals: 0,
        };
        for ((n, _), t) in f.params.iter().zip(&param_types) {
            let i = l.local();
            l.vars.insert(n.clone(), (i, *t));
        }
        let (block, ty) = l.block(&f.body)?;
        let mut body = block.stmts;
        match ty {
            None => {}
            Some(_) if result_type == ValueType::Unit => eval(&mut body, block.value),
            Some(t) if t == result_type => body.push(Stmt::Return(Some(block.value))),
            Some(_) => return Err(format!("IR は関数 {} の戻り値に未対応", f.name).into()),
        }
        let function = Function {
            name: f.name.clone(),
            param_types,
            result_type,
            locals: l.locals,
            body,
        };
        for callee in function.calls() {
            pending.push(sources[callee].1.name.as_str());
        }
        functions.push(function);
    }
    functions.sort_by_key(|f| sources[f.name.as_str()].0);
    imports.sort_by_key(|f| sources[f.name.as_str()].0);
    Ok(Program { functions, imports })
}

struct Lower<'a> {
    signatures: &'a HashMap<&'a str, Signature>,
    vars: HashMap<String, (u32, ValueType)>,
    locals: u32,
}

// 式の型。None は値を返さずに抜ける（return を通る）
type Ty = Option<ValueType>;

fn join(a: Ty, b: Ty) -> R<Ty> {
    match (a, b) {
        (None, t) | (t, None) => Ok(t),
        (Some(a), Some(b)) if a == b => Ok(Some(a)),
        _ => Err("IR は分岐で型の違う if に未対応".into()),
    }
}

// 値を捨てる。unit の塊は中の文を並べ直す
fn eval(out: &mut Vec<Stmt>, e: Expr) {
    match e {
        Expr::Int(0) => {}
        Expr::Block(b) if b.value == Expr::Int(0) => out.extend(b.stmts),
        e => out.push(Stmt::Eval(e)),
    }
}

fn unit(stmts: Vec<Stmt>) -> Expr {
    Expr::Block(Box::new(Block {
        stmts,
        value: Expr::Int(0),
    }))
}

impl Lower<'_> {
    fn local(&mut self) -> u32 {
        self.locals += 1;
        self.locals - 1
    }

    fn block(&mut self, b: &parser::Block) -> R<(Block, Ty)> {
        let saved = self.vars.clone();
        let mut stmts = Vec::new();
        for s in &b.stmts {
            match s {
                AstStmt::Let { name, init, .. } => {
                    let (e, t) = self.expr(init)?;
                    let t = t.ok_or("IR は return を初期値に持つ let に未対応")?;
                    let i = self.local();
                    stmts.push(Stmt::Set(i, e));
                    self.vars.insert(name.clone(), (i, t));
                }
                AstStmt::Expr(e) => self.effect(e, &mut stmts)?,
            }
        }
        let (value, ty) = match &b.tail {
            Some(e) => self.expr(e)?,
            None if matches!(stmts.last(), Some(Stmt::Return(_))) => (Expr::Int(0), None),
            None => (Expr::Int(0), Some(ValueType::Unit)),
        };
        self.vars = saved;
        Ok((Block { stmts, value }, ty))
    }

    // 文の位置の式。値は捨てる
    fn effect(&mut self, e: &Ast, out: &mut Vec<Stmt>) -> R<()> {
        match e.peel() {
            Ast::Call(n, args) if n == "print" && args.len() == 1 => {
                let (v, t) = self.expr(&args[0])?;
                out.push(match t.ok_or("IR は print の引数の return に未対応")? {
                    ValueType::Int => Stmt::Print(ValueType::Int, v),
                    ValueType::Str => Stmt::Print(ValueType::Str, v),
                    ValueType::Bool => Stmt::Print(
                        ValueType::Str,
                        Expr::If(
                            Box::new(v),
                            Box::new(Block {
                                stmts: vec![],
                                value: Expr::Str("true".into()),
                            }),
                            Box::new(Block {
                                stmts: vec![],
                                value: Expr::Str("false".into()),
                            }),
                        ),
                    ),
                    ValueType::Unit => return Err("IR は unit の print に未対応".into()),
                });
            }
            Ast::Assign(n, v) => {
                let (i, t) = *self
                    .vars
                    .get(n)
                    .ok_or_else(|| format!("IR は変数 {n} に未対応"))?;
                let (v, vt) = self.expr(v)?;
                join(Some(t), vt)?;
                out.push(Stmt::Set(i, v));
            }
            Ast::While(c, body) => {
                let (c, _) = self.expr(c)?;
                let (body, _) = self.block(body)?;
                let mut stmts = body.stmts;
                eval(&mut stmts, body.value);
                out.push(Stmt::While(c, stmts));
            }
            Ast::For(x, from, Some(to), body) => {
                let (from, _) = self.expr(from)?;
                let (to, _) = self.expr(to)?;
                let (i, end) = (self.local(), self.local());
                out.push(Stmt::Set(i, from));
                out.push(Stmt::Set(end, to));
                let saved = self.vars.clone();
                self.vars.insert(x.clone(), (i, ValueType::Int));
                let (body, _) = self.block(body)?;
                self.vars = saved;
                let mut stmts = body.stmts;
                eval(&mut stmts, body.value);
                let next = Expr::Binary(Op::Add, Box::new(Expr::Local(i)), Box::new(Expr::Int(1)));
                stmts.push(Stmt::Set(i, next));
                let c = Expr::Binary(Op::Lt, Box::new(Expr::Local(i)), Box::new(Expr::Local(end)));
                out.push(Stmt::While(c, stmts));
            }
            Ast::Return(v) => {
                let v = v.as_ref().map(|v| self.expr(v)).transpose()?;
                out.push(Stmt::Return(v.map(|(v, _)| v)));
            }
            _ => {
                let (v, _) = self.expr(e)?;
                eval(out, v);
            }
        }
        Ok(())
    }

    fn expr(&mut self, e: &Ast) -> R<(Expr, Ty)> {
        use ValueType::*;
        Ok(match e.peel() {
            Ast::Int(n) => (Expr::Int(*n), Some(Int)),
            Ast::Bool(b) => (Expr::Int(*b as i64), Some(Bool)),
            Ast::Str(s) => (Expr::Str(s.clone()), Some(Str)),
            Ast::Var(n) => {
                let (i, t) = *self
                    .vars
                    .get(n)
                    .ok_or_else(|| format!("IR は変数 {n} に未対応"))?;
                (Expr::Local(i), Some(t))
            }
            Ast::Unary("-", a) => {
                let (a, _) = self.expr(a)?;
                (
                    Expr::Binary(Op::Sub, Box::new(Expr::Int(0)), Box::new(a)),
                    Some(Int),
                )
            }
            Ast::Unary("!", a) => (Expr::Not(Box::new(self.expr(a)?.0)), Some(Bool)),
            Ast::Binary(op @ ("&&" | "||"), a, b) => {
                let (a, _) = self.expr(a)?;
                let (b, _) = self.expr(b)?;
                let leaf = |value| {
                    Box::new(Block {
                        stmts: vec![],
                        value,
                    })
                };
                let (t, f) = if *op == "&&" {
                    (leaf(b), leaf(Expr::Int(0)))
                } else {
                    (leaf(Expr::Int(1)), leaf(b))
                };
                (Expr::If(Box::new(a), t, f), Some(Bool))
            }
            Ast::Binary(op, a, b) => {
                let (a, at) = self.expr(a)?;
                let (b, bt) = self.expr(b)?;
                let (op, ty) = match *op {
                    "+" => (Op::Add, Int),
                    "-" => (Op::Sub, Int),
                    "*" => (Op::Mul, Int),
                    "/" => (Op::Div, Int),
                    "%" => (Op::Rem, Int),
                    "==" => (Op::Eq, Bool),
                    "!=" => (Op::Ne, Bool),
                    "<" => (Op::Lt, Bool),
                    "<=" => (Op::Le, Bool),
                    ">" => (Op::Gt, Bool),
                    ">=" => (Op::Ge, Bool),
                    op => return Err(format!("IR は演算子 {op} に未対応").into()),
                };
                let operand = join(at, bt)?;
                let ok = match op {
                    Op::Eq | Op::Ne => matches!(operand, Some(Int | Bool)),
                    _ => operand == Some(Int),
                };
                if !ok {
                    return Err("IR は整数と真偽値以外の演算に未対応".into());
                }
                (Expr::Binary(op, Box::new(a), Box::new(b)), Some(ty))
            }
            Ast::Call(n, args) if n == "print" && args.len() == 1 => {
                let mut stmts = Vec::new();
                self.effect(e, &mut stmts)?;
                (unit(stmts), Some(Unit))
            }
            Ast::Call(n, args) => {
                let Some(Some((params, result))) = self.signatures.get(n.as_str()) else {
                    return Err(format!("IR は関数 {n} の呼び出しに未対応").into());
                };
                if params.len() != args.len() {
                    return Err(format!("IR は関数 {n} の省略した引数に未対応").into());
                }
                let result = *result;
                let args = args
                    .iter()
                    .map(|a| self.expr(a).map(|(a, _)| a))
                    .collect::<R<Vec<_>>>()?;
                (Expr::Call(n.clone(), args), Some(result))
            }
            Ast::If(c, t, f) => {
                let (c, _) = self.expr(c)?;
                let (t, tt) = self.block(t)?;
                let (f, ft) = match f {
                    Some(f) => {
                        let (f, ft) = self.expr(f)?;
                        let f = match f {
                            Expr::Block(b) => *b,
                            value => Block {
                                stmts: vec![],
                                value,
                            },
                        };
                        (f, ft)
                    }
                    None => (
                        Block {
                            stmts: vec![],
                            value: Expr::Int(0),
                        },
                        Some(Unit),
                    ),
                };
                // else の無い if は unit。then の値は捨てる
                let ty = if f.value == Expr::Int(0) && ft == Some(Unit) && tt.is_some() {
                    Some(Unit)
                } else {
                    join(tt, ft)?
                };
                (Expr::If(Box::new(c), Box::new(t), Box::new(f)), ty)
            }
            Ast::Block(b) => {
                let (b, t) = self.block(b)?;
                (Expr::Block(Box::new(b)), t)
            }
            Ast::Assign(..) | Ast::While(..) | Ast::For(..) | Ast::Return(_) => {
                let mut stmts = Vec::new();
                self.effect(e, &mut stmts)?;
                let diverges = matches!(e.peel(), Ast::Return(_));
                (unit(stmts), if diverges { None } else { Some(Unit) })
            }
            _ => return Err("IR はこの式に未対応".into()),
        })
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for fun in &self.functions {
            write!(f, "fn {}(", fun.name)?;
            for (i, t) in fun.param_types.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "%{i}: {t}")?;
            }
            f.write_str(")")?;
            if fun.result_type != ValueType::Unit {
                write!(f, " -> {}", fun.result_type)?;
            }
            writeln!(f, " {{")?;
            stmts(f, &fun.body, 1)?;
            writeln!(f, "}}")?;
        }
        Ok(())
    }
}

fn stmts(f: &mut fmt::Formatter<'_>, ss: &[Stmt], depth: usize) -> fmt::Result {
    let pad = "  ".repeat(depth);
    for s in ss {
        match s {
            Stmt::Set(i, e) => writeln!(f, "{pad}%{i} = {e}")?,
            Stmt::Eval(e) => writeln!(f, "{pad}{e}")?,
            Stmt::Print(t, e) => writeln!(f, "{pad}print {t} {e}")?,
            Stmt::Return(Some(e)) => writeln!(f, "{pad}return {e}")?,
            Stmt::Return(None) => writeln!(f, "{pad}return")?,
            Stmt::While(c, body) => {
                writeln!(f, "{pad}while {c} {{")?;
                stmts(f, body, depth + 1)?;
                writeln!(f, "{pad}}}")?;
            }
        }
    }
    Ok(())
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.stmts.is_empty() {
            return write!(f, "{}", self.value);
        }
        writeln!(f, "{{")?;
        stmts(f, &self.stmts, 2)?;
        write!(f, "    {} }}", self.value)
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Int(n) => write!(f, "{n}"),
            Expr::Str(s) => write!(f, "{s:?}"),
            Expr::Local(i) => write!(f, "%{i}"),
            Expr::Binary(op, a, b) => write!(f, "({op:?} {a} {b})"),
            Expr::Not(a) => write!(f, "(not {a})"),
            Expr::Call(n, args) => {
                write!(f, "(call {n}")?;
                for a in args {
                    write!(f, " {a}")?;
                }
                f.write_str(")")
            }
            Expr::If(c, t, e) => write!(f, "(if {c} {t} {e})"),
            Expr::Block(b) => write!(f, "{b}"),
        }
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
            "fn add(%0: int, %1: int) -> int {\n  return (Add %0 %1)\n}\nfn main() {\n  %0 = (call add 1 2)\n  %1 = (Mul %0 3)\n  (Add %1 4)\n}\n"
        );
    }

    #[test]
    fn lowers_control_flow() {
        let src = "fn f(n: int) -> bool {\n let mut i = 0\n while i < n { if i == 3 { return true }\n i = i + 1 }\n false\n}\nfn main() { print(f(5)) }";
        let ir = crate::to_ir(src).unwrap();
        assert!(ir.contains("while (Lt %1 %0) {"), "{ir}");
        assert!(ir.contains("(if (Eq %1 3) {"), "{ir}");
        assert!(
            ir.contains("print str (if (call f 5) \"true\" \"false\")"),
            "{ir}"
        );
    }

    #[test]
    fn rejects_unsupported_and_invalid_programs() {
        assert!(
            crate::to_ir("fn main() { let x = [1] }")
                .unwrap_err()
                .contains("未対応")
        );
        assert!(crate::to_ir("fn main() { let x = 1 + true }").is_err());
    }
}
