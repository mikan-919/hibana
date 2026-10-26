use crate::parser::{Block, Expr, Fn, Stmt};
use std::collections::HashMap;
use std::io::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Unit,
    Str(std::rc::Rc<str>),
}

// 通常の値以外で評価を抜ける理由
enum Flow {
    Return(Value),
    Error(String),
}

type R<T> = Result<T, Flow>;

// ponytail: 構文木を直接評価する方式。速度が必要ならバイトコードVMに置き換える
struct Interp<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    out: &'a mut dyn Write,
}

// 型検査を通ったプログラムを前提とする
pub fn run(fns: &[Fn], out: &mut dyn Write) -> Result<Value, String> {
    let mut it = Interp {
        fns: fns.iter().map(|f| (f.name.as_str(), f)).collect(),
        out,
    };
    match it.call("main", vec![]) {
        Ok(v) | Err(Flow::Return(v)) => Ok(v),
        Err(Flow::Error(e)) => Err(e),
    }
}

fn int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        _ => unreachable!("型検査済み"),
    }
}

fn boolean(v: Value) -> bool {
    match v {
        Value::Bool(b) => b,
        _ => unreachable!("型検査済み"),
    }
}

fn arith(op: &str, a: i64, b: i64) -> Option<i64> {
    match op {
        "+" => a.checked_add(b),
        "-" => a.checked_sub(b),
        "*" => a.checked_mul(b),
        "/" => a.checked_div(b),
        _ => a.checked_rem(b),
    }
}

impl Interp<'_> {
    fn call(&mut self, name: &str, args: Vec<Value>) -> R<Value> {
        let f = self.fns[name];
        let mut env = vec![f.params.iter().map(|(n, _)| n.clone()).zip(args).collect()];
        match self.block(&f.body, &mut env) {
            Err(Flow::Return(v)) => Ok(v),
            r => r,
        }
    }

    fn block(&mut self, b: &Block, env: &mut Vec<HashMap<String, Value>>) -> R<Value> {
        env.push(HashMap::new());
        let r = self.block_inner(b, env);
        env.pop();
        r
    }

    fn block_inner(&mut self, b: &Block, env: &mut Vec<HashMap<String, Value>>) -> R<Value> {
        for s in &b.stmts {
            match s {
                Stmt::Let { name, init, .. } => {
                    let v = self.expr(init, env)?;
                    env.last_mut().unwrap().insert(name.clone(), v);
                }
                Stmt::Expr(e) => {
                    self.expr(e, env)?;
                }
            }
        }
        match &b.tail {
            Some(e) => self.expr(e, env),
            None => Ok(Value::Unit),
        }
    }

    fn expr(&mut self, e: &Expr, env: &mut Vec<HashMap<String, Value>>) -> R<Value> {
        Ok(match e {
            Expr::Int(n) => Value::Int(*n),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Str(s) => Value::Str(s.as_str().into()),
            Expr::Field(..) | Expr::Method(..) | Expr::Macro(..) => unreachable!("型検査で弾く"),
            Expr::Var(n) => env.iter().rev().find_map(|s| s.get(n)).unwrap().clone(),
            Expr::Unary("-", x) => {
                let n = int(self.expr(x, env)?);
                Value::Int(
                    n.checked_neg()
                        .ok_or(Flow::Error("オーバーフロー".into()))?,
                )
            }
            Expr::Unary(_, x) => Value::Bool(!boolean(self.expr(x, env)?)),
            Expr::Binary("&&", l, r) => {
                Value::Bool(boolean(self.expr(l, env)?) && boolean(self.expr(r, env)?))
            }
            Expr::Binary("||", l, r) => {
                Value::Bool(boolean(self.expr(l, env)?) || boolean(self.expr(r, env)?))
            }
            Expr::Binary(op, l, r) => {
                let (a, b) = (self.expr(l, env)?, self.expr(r, env)?);
                match *op {
                    "==" => Value::Bool(a == b),
                    "!=" => Value::Bool(a != b),
                    "<" => Value::Bool(int(a) < int(b)),
                    "<=" => Value::Bool(int(a) <= int(b)),
                    ">" => Value::Bool(int(a) > int(b)),
                    ">=" => Value::Bool(int(a) >= int(b)),
                    _ => {
                        let (a, b) = (int(a), int(b));
                        if b == 0 && matches!(*op, "/" | "%") {
                            return Err(Flow::Error("0 で除算した".into()));
                        }
                        Value::Int(arith(op, a, b).ok_or(Flow::Error("オーバーフロー".into()))?)
                    }
                }
            }
            Expr::Call(name, args) => {
                let mut vs = Vec::new();
                for a in args {
                    vs.push(self.expr(a, env)?);
                }
                if name == "print" {
                    let s = match &vs[0] {
                        Value::Int(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Unit => "()".into(),
                        Value::Str(s) => s.to_string(),
                    };
                    writeln!(self.out, "{s}").map_err(|e| Flow::Error(e.to_string()))?;
                    Value::Unit
                } else {
                    self.call(name, vs)?
                }
            }
            Expr::Assign(name, x) => {
                let v = self.expr(x, env)?;
                *env.iter_mut().rev().find_map(|s| s.get_mut(name)).unwrap() = v;
                Value::Unit
            }
            Expr::If(c, then, els) => {
                if boolean(self.expr(c, env)?) {
                    self.block(then, env)?
                } else if let Some(x) = els {
                    self.expr(x, env)?
                } else {
                    Value::Unit
                }
            }
            Expr::While(c, body) => {
                while boolean(self.expr(c, env)?) {
                    self.block(body, env)?;
                }
                Value::Unit
            }
            Expr::Block(b) => self.block(b, env)?,
            Expr::Return(x) => {
                let v = match x {
                    Some(x) => self.expr(x, env)?,
                    None => Value::Unit,
                };
                return Err(Flow::Return(v));
            }
        })
    }
}
