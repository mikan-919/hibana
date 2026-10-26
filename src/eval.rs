use crate::error::Error;
use crate::lexer::{Span, Tok, Token};
use crate::meta::{self, Node, Syn};
use crate::parser::{Block, Expr, Fn, Program, Stmt};
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Unit,
    Str(Rc<str>),
    // コード片。マクロ本体の中でだけ現れる
    Syn(Rc<Syn>),
    // 型。実行時には名前だけを持つ
    Ty(Rc<str>),
    Arr(Rc<Vec<Value>>),
    // data の値 (構築子, フィールド)
    Data(Rc<(String, Vec<Value>)>),
    Fn(Rc<Callable>),
}

// 関数の値
#[derive(Debug)]
pub enum Callable {
    // ponytail: 作った時の変数を値で写し取る。後から代入しても関数の中には届かない
    Lam {
        params: Vec<String>,
        body: Block,
        env: HashMap<String, Value>,
    },
    // トップレベルの関数
    Named(String),
}

// 関数の値は同じものの時だけ等しい（型検査で比較を禁止していないので、ここで決める）
impl PartialEq for Callable {
    fn eq(&self, o: &Self) -> bool {
        std::ptr::eq(self, o)
    }
}

// 通常の値以外で評価を抜ける理由
enum Flow {
    Return(Value),
    Error(String),
    // handle の腕が resume しなかった。(handle の番号, handle の値)
    Abort(usize, Value),
}

type R<T> = Result<T, Flow>;

// ponytail: 構文木を直接評価する方式。速度が必要ならバイトコードVMに置き換える
struct Interp<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    // 構築子の名前からフィールド名の並び
    ctors: HashMap<&'a str, Vec<&'a str>>,
    out: &'a mut dyn Write,
    // 処理中の handle (番号, 操作の名前から腕の関数)。内側ほど後ろ
    handlers: Vec<(usize, HashMap<String, Value>)>,
    next_h: usize,
}

// 型検査を通ったプログラムを前提とする
pub fn run(prog: &Program, out: &mut dyn Write) -> Result<Value, String> {
    let mut it = Interp {
        fns: prog.fns.iter().map(|f| (f.name.as_str(), f)).collect(),
        ctors: prog
            .datas
            .iter()
            .flat_map(|d| &d.ctors)
            .map(|c| {
                (
                    c.name.as_str(),
                    c.fields.iter().map(|f| f.0.as_str()).collect(),
                )
            })
            .collect(),
        out,
        handlers: Vec::new(),
        next_h: 0,
    };
    match it.call("main", vec![]) {
        Ok(v) | Err(Flow::Return(v)) => Ok(v),
        Err(Flow::Error(e)) => Err(e),
        Err(Flow::Abort(..)) => unreachable!("handle の外には出ない"),
    }
}

// マクロ本体を実行する。print はコンパイル時の出力として標準エラーに書く
pub fn run_macro(b: &Block, vars: Vec<(String, Value)>) -> Result<Value, String> {
    let mut err = std::io::stderr();
    let mut it = Interp {
        fns: HashMap::new(),
        ctors: HashMap::new(),
        out: &mut err,
        handlers: Vec::new(),
        next_h: 0,
    };
    match it.block(b, &mut vec![vars.into_iter().collect()]) {
        Ok(v) | Err(Flow::Return(v)) => Ok(v),
        Err(Flow::Error(e)) => Err(e),
        Err(Flow::Abort(..)) => unreachable!("handle の外には出ない"),
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::Arr(xs) => format!("[{}]", xs.iter().map(show).collect::<Vec<_>>().join(", ")),
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => format!("{s:?}"),
        Value::Ty(t) => t.to_string(),
        Value::Unit => "()".into(),
        Value::Syn(s) => format!("{:?}", s.node.strip()),
        Value::Fn(_) => "<fn>".into(),
        Value::Data(d) if d.1.is_empty() => d.0.clone(),
        Value::Data(d) => format!(
            "{}({})",
            d.0,
            d.1.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn syn(v: Value) -> Rc<Syn> {
    match v {
        Value::Syn(s) => s,
        _ => unreachable!("型検査済み"),
    }
}

fn str_val(v: Value) -> Rc<str> {
    match v {
        Value::Str(s) => s,
        _ => unreachable!("型検査済み"),
    }
}

fn error<T>(msg: String) -> R<T> {
    Err(Flow::Error(msg))
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
    // マクロ本体用の組み込み関数。該当しなければ None
    fn syn_fn(&mut self, name: &str, vs: &mut Vec<Value>) -> R<Option<Value>> {
        if !matches!(
            name,
            "len"
                | "at"
                | "is_null"
                | "int_of"
                | "str_of"
                | "error"
                | "Vec"
                | "to_str"
                | "split"
                | "starts_with"
                | "slice"
                | "ident"
        ) || self.fns.contains_key(name)
        {
            return Ok(None);
        }
        let a = vs.remove(0);
        match (name, &a) {
            ("error", _) => return error(str_val(a).to_string()),
            ("Vec", _) => return Ok(Some(Value::Ty("Vec".into()))),
            ("len", Value::Arr(xs)) => return Ok(Some(Value::Int(xs.len() as i64))),
            ("len", Value::Str(s)) => return Ok(Some(Value::Int(s.chars().count() as i64))),
            ("to_str", Value::Str(_)) => return Ok(Some(a)),
            ("to_str", v) => return Ok(Some(Value::Str(self.show(v).into()))),
            ("split", Value::Str(s)) => {
                let sep = str_val(vs.remove(0));
                return Ok(Some(Value::Arr(Rc::new(
                    s.split(&*sep).map(|p| Value::Str(p.into())).collect(),
                ))));
            }
            ("starts_with", Value::Str(s)) => {
                return Ok(Some(Value::Bool(s.starts_with(&*str_val(vs.remove(0))))));
            }
            ("slice", Value::Str(s)) => {
                let (i, j) = (int(vs.remove(0)), int(vs.remove(0)));
                let cs: Vec<char> = s.chars().collect();
                if !(0 <= i && i <= j && j as usize <= cs.len()) {
                    return error(format!("slice の範囲 {i}..{j} が文字列の外"));
                }
                return Ok(Some(Value::Str(
                    cs[i as usize..j as usize].iter().collect::<String>().into(),
                )));
            }
            ("ident", Value::Str(s)) => {
                let tok = Token {
                    tok: Tok::Ident(s.to_string()),
                    span: Span::default(),
                    gap_before: true,
                    nl_before: false,
                };
                let node = Node::At(0..1, Box::new(Node::Ident(s.to_string(), Span::default())));
                return Ok(Some(Value::Syn(Rc::new(Syn {
                    node,
                    toks: Rc::new(vec![tok]),
                }))));
            }
            _ => {}
        }
        let s = syn(a);
        let n = s.node.strip();
        Ok(Some(match (name, n) {
            ("len", Node::List(v)) => Value::Int(v.len() as i64),
            ("at", Node::List(v)) => {
                let i = int(vs.remove(0));
                match usize::try_from(i).ok().and_then(|i| v.get(i)) {
                    Some(x) => Value::Syn(Rc::new(s.child(x.clone()))),
                    None => return error(format!("添字 {i} が範囲外")),
                }
            }
            ("is_null", n) => Value::Bool(matches!(n, Node::Null)),
            ("int_of", Node::Int(i)) => Value::Int(*i),
            ("str_of", Node::Str(x) | Node::Ident(x, _) | Node::Text(x, _) | Node::Token(x, _)) => {
                Value::Str(x.as_str().into())
            }
            _ => return error(format!("{name} を {n:?} に使えない")),
        }))
    }

    // 値を表示用の文字列にする。構造体はフィールド名を付けて表す
    fn show(&self, v: &Value) -> String {
        match v {
            Value::Arr(xs) => format!(
                "[{}]",
                xs.iter()
                    .map(|x| self.show(x))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Value::Data(d) if d.0.ends_with("{}") => {
                let fs = &self.ctors[d.0.as_str()];
                let body: Vec<String> = fs
                    .iter()
                    .zip(&d.1)
                    .map(|(f, x)| format!("{f}: {}", self.show(x)))
                    .collect();
                format!("{} {{ {} }}", &d.0[..d.0.len() - 2], body.join(", "))
            }
            Value::Data(d) if !d.1.is_empty() => {
                format!(
                    "{}({})",
                    d.0,
                    d.1.iter()
                        .map(|x| self.show(x))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            v => show(v),
        }
    }

    // 関数の値を呼ぶ
    fn apply(&mut self, f: &Value, args: Vec<Value>) -> R<Value> {
        let Value::Fn(c) = f else {
            unreachable!("型検査済み")
        };
        match &**c {
            Callable::Named(n) => self.call(n, args),
            Callable::Lam { params, body, env } => {
                let mut scope = env.clone();
                scope.extend(params.iter().cloned().zip(args));
                match self.block(body, &mut vec![scope]) {
                    Err(Flow::Return(v)) => Ok(v),
                    r => r,
                }
            }
        }
    }

    fn call(&mut self, name: &str, args: Vec<Value>) -> R<Value> {
        let f = self.fns[name];
        if f.op.is_some() {
            let k = args.len() - (f.params.len() - f.implicit);
            return self.perform(name, args[k..].to_vec());
        }
        if f.js.is_some() {
            return error(format!(
                "extern の関数 {name} は JS ターゲット（hibana js）でだけ使える"
            ));
        }
        // 型検査で推論したジェネリック引数は実行時には消えている
        // ponytail: 型引数を値として使うと "?" になる。必要なら型検査の結果を構文木に書き戻す
        let args = if args.len() + f.implicit == f.params.len() {
            let mut full = vec![Value::Ty("?".into()); f.implicit];
            full.extend(args);
            full
        } else {
            args
        };
        let mut env = vec![f.params.iter().map(|(n, _)| n.clone()).zip(args).collect()];
        match self.block(&f.body, &mut env) {
            Err(Flow::Return(v)) => Ok(v),
            r => r,
        }
    }

    // 操作を、それを処理する一番内側の handle の腕で実行する。腕の中の操作は、その handle より外で処理する
    fn perform(&mut self, op: &str, args: Vec<Value>) -> R<Value> {
        let Some(i) = self.handlers.iter().rposition(|h| h.1.contains_key(op)) else {
            return error(format!("操作 {op} を処理する handle が無い"));
        };
        let (id, f) = (self.handlers[i].0, self.handlers[i].1[op].clone());
        let saved = self.handlers.split_off(i);
        let r = self.apply(&f, args);
        self.handlers.extend(saved);
        match r? {
            Value::Data(d) if d.0 == "$resume" => Ok(d.1[0].clone()),
            v => Err(Flow::Abort(id, v)),
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
            Expr::At(_, x) => self.expr(x, env)?,
            Expr::Int(n) => Value::Int(*n),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Str(s) => Value::Str(s.as_str().into()),
            Expr::Method(..) | Expr::Macro(..) => unreachable!("型検査で弾く"),
            // 細分化型は実行時には名前だけを持つ型
            Expr::Refine(..) => Value::Ty("refine".into()),
            Expr::Array(items) => {
                let mut vs = Vec::new();
                for a in items {
                    vs.push(self.expr(a, env)?);
                }
                Value::Arr(Rc::new(vs))
            }
            Expr::Index(v, i) => {
                let Value::Arr(xs) = self.expr(v, env)? else {
                    unreachable!("型検査済み")
                };
                let i = int(self.expr(i, env)?);
                // 範囲は型検査で証明済み
                xs[i as usize].clone()
            }
            Expr::Field(x, name) => {
                let v = self.expr(x, env)?;
                if let Value::Data(d) = &v {
                    let i = self.ctors[d.0.as_str()]
                        .iter()
                        .position(|f| f == name)
                        .expect("型検査済み");
                    return Ok(d.1[i].clone());
                }
                let s = syn(v);
                match s.node.get(name) {
                    Some(n) => Value::Syn(Rc::new(s.child(n.clone()))),
                    None => return error(format!("フィールド {name} が無い")),
                }
            }
            Expr::Quote(q) => {
                let mut flat: Vec<(String, Value)> = env
                    .iter()
                    .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect();
                let mut out = Vec::new();
                let mut ev = |e: &Expr, vars: &[(String, Value)]| {
                    env.push(vars.iter().cloned().collect());
                    let r = self.expr(e, env);
                    env.pop();
                    r.map_err(|f| match f {
                        Flow::Error(e) => Error::from(e),
                        Flow::Return(_) => "${ } の中で return した".into(),
                        Flow::Abort(..) => unreachable!("マクロ本体に handle は無い"),
                    })
                };
                meta::splice(q, &mut flat, &mut out, &mut ev).map_err(|e| Flow::Error(e.msg))?;
                let n = out.len();
                Value::Syn(Rc::new(Syn {
                    node: Node::At(0..n, Box::new(Node::Raw)),
                    toks: Rc::new(out),
                }))
            }
            Expr::Var(n) => match env.iter().rev().find_map(|s| s.get(n)) {
                Some(v) => v.clone(),
                None if self
                    .fns
                    .get(n.as_str())
                    .is_some_and(|f| f.alias && f.params.is_empty()) =>
                {
                    self.call(n, vec![])?
                }
                None if self.fns.contains_key(n.as_str()) => {
                    Value::Fn(Rc::new(Callable::Named(n.clone())))
                }
                None if self.ctors.contains_key(n.as_str()) => {
                    Value::Data(Rc::new((n.clone(), vec![])))
                }
                // 型検査を通っているので、残りは int などの型名
                None => Value::Ty(n.as_str().into()),
            },
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
                    "+" if matches!(a, Value::Str(_)) => {
                        Value::Str(format!("{}{}", str_val(a), str_val(b)).into())
                    }
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
                if let Some(f) = env.iter().rev().find_map(|s| s.get(name)).cloned() {
                    self.apply(&f, vs)?
                } else if name == "all" && !self.fns.contains_key(name.as_str()) {
                    // 並行の処理が無いので順に実行する
                    let Value::Arr(fs) = &vs[0] else {
                        unreachable!("型検査済み")
                    };
                    let mut out = Vec::new();
                    for f in fs.iter() {
                        out.push(self.apply(f, vec![])?);
                    }
                    Value::Arr(Rc::new(out))
                } else if name == "resume" {
                    // 腕の値として返し、perform が操作の戻り値にする
                    Value::Data(Rc::new((
                        "$resume".into(),
                        vec![vs.pop().unwrap_or(Value::Unit)],
                    )))
                } else if self.ctors.contains_key(name.as_str()) {
                    Value::Data(Rc::new((name.clone(), vs)))
                } else if !self.fns.contains_key(name.as_str())
                    && name != "print"
                    && name.starts_with(|c: char| c.is_ascii_uppercase())
                {
                    // 型を作る呼び出し（Option(int) など）。実行時には名前だけを持つ
                    Value::Ty(name.as_str().into())
                } else if let Some(v) = self.syn_fn(name, &mut vs)? {
                    v
                } else if name == "print" {
                    let s = match &vs[0] {
                        Value::Int(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Unit => "()".into(),
                        Value::Str(s) => s.to_string(),
                        Value::Ty(t) => t.to_string(),
                        Value::Arr(_) | Value::Data(_) | Value::Fn(_) => self.show(&vs[0]),
                        Value::Syn(s) => format!("{:?}", s.node.strip()),
                    };
                    writeln!(self.out, "{s}").map_err(|e| Flow::Error(e.to_string()))?;
                    Value::Unit
                } else {
                    self.call(name, vs)?
                }
            }
            Expr::Lam(params, _, body) => Value::Fn(Rc::new(Callable::Lam {
                params: params.iter().map(|p| p.0.clone()).collect(),
                body: body.clone(),
                env: env
                    .iter()
                    .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect(),
            })),
            Expr::Pi(..) => Value::Ty("fn".into()),
            Expr::Handle(body, arms) => {
                self.next_h += 1;
                let id = self.next_h;
                let captured: HashMap<String, Value> = env
                    .iter()
                    .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect();
                let arms = arms
                    .iter()
                    .map(|a| {
                        let body = Block {
                            stmts: vec![],
                            tail: Some(Box::new(a.body.clone())),
                        };
                        let f = Callable::Lam {
                            params: a.binds.clone(),
                            body,
                            env: captured.clone(),
                        };
                        (a.ctor.clone(), Value::Fn(Rc::new(f)))
                    })
                    .collect();
                let n = self.handlers.len();
                self.handlers.push((id, arms));
                let r = self.expr(body, env);
                self.handlers.truncate(n);
                match r {
                    Err(Flow::Abort(h, v)) if h == id => v,
                    r => r?,
                }
            }
            Expr::Apply(f, args) => {
                let f = self.expr(f, env)?;
                let mut vs = Vec::new();
                for a in args {
                    vs.push(self.expr(a, env)?);
                }
                self.apply(&f, vs)?
            }
            Expr::Match(s, arms) => {
                let Value::Data(d) = self.expr(s, env)? else {
                    unreachable!("型検査済み")
                };
                let arm = arms
                    .iter()
                    .find(|a| a.ctor == d.0 || a.ctor == "_")
                    .expect("網羅性は型検査済み");
                let mut scope = HashMap::new();
                if arm.ctor != "_" {
                    scope.extend(arm.binds.iter().cloned().zip(d.1.iter().cloned()));
                }
                env.push(scope);
                let r = self.expr(&arm.body, env);
                env.pop();
                r?
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
            Expr::For(x, it, end, body) => {
                let items: Vec<Value> = match end {
                    Some(end) => (int(self.expr(it, env)?)..int(self.expr(end, env)?))
                        .map(Value::Int)
                        .collect(),
                    None => match self.expr(it, env)? {
                        Value::Arr(xs) => xs.to_vec(),
                        _ => unreachable!("型検査済み"),
                    },
                };
                for v in items {
                    env.push(HashMap::from([(x.clone(), v)]));
                    let r = self.block(body, env);
                    env.pop();
                    r?;
                }
                Value::Unit
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
