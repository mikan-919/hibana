use crate::parser::Name;
use crate::check::typeck::{MIN_SAFE_INT, MAX_SAFE_INT};
use crate::error::Error;
use crate::lexer::{Span, Tok, Token};
use crate::meta::{self, Node, Syn};
use crate::parser::{Block, Expr, Fn, NodeId, Program, Stmt};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll, Waker};

mod jobs;
use jobs::{Jobs, Promise};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Num(f64),
    Bool(bool),
    Unit,
    Str(Rc<str>),
    // コード片。マクロ本体の中でだけ現れる
    Syn(Rc<Syn>),
    // 型。実行時には名前だけを持つ
    Ty(Rc<str>),
    Arr(Rc<Vec<Value>>),
    // data の値 (構築子, フィールド)
    Data(Rc<(Name, Vec<Value>)>),
    Fn(Rc<Callable>),
    Task(Rc<Task>),
    // 書き換えられる参照。写し取った関数どうしでも同じものを指す
    Ref(Rc<std::cell::RefCell<Value>>),
}

// 関数の値
#[derive(Debug)]
pub enum Callable {
    // ponytail: 作った時の変数を値で写し取る。後から代入しても関数の中には届かない
    Lam {
        params: Vec<Name>,
        body: Block,
        env: HashMap<Name, Value>,
        variant: bool,
    },
    // トップレベルの関数
    Named(Name),
}

// Rust 側で関数の値を比較する場合は同一性を使う。
impl PartialEq for Callable {
    fn eq(&self, o: &Self) -> bool {
        std::ptr::eq(self, o)
    }
}

#[derive(Debug)]
pub struct Task(Promise);

impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

// 通常の値以外で評価を抜ける理由
#[derive(Debug, Clone)]
enum Flow {
    Return(Value),
    Break,
    Continue,
    Error(String, Option<Span>),
    // handle の腕が resume しなかった。(handle の番号, handle の値)
    Abort(usize, Value),
}

type R<T> = Result<T, Flow>;

struct Runtime<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    // 構築子の名前からフィールド名の並び
    ctors: HashMap<&'a str, (&'a Name, Vec<&'a Name>)>,
    out: RefCell<&'a mut dyn Write>,
    info: &'a crate::typeck::Info,
    next_h: Cell<usize>,
    callback_error: RefCell<Option<Flow>>,
}

// 構文木を直接評価する。待っている関数は局所変数と with の記録を持ったまま止まる
struct Interp<'a> {
    runtime: Rc<Runtime<'a>>,
    jobs: Weak<Jobs<'a>>,
    // 処理中の handle (番号, 操作の名前から腕の関数)。内側ほど後ろ
    handlers: Vec<(usize, Rc<HashMap<Name, Value>>)>,
    variant: bool,
}

// 型検査を通ったプログラムを前提とする
pub fn run(prog: &Program, out: &mut dyn Write) -> Result<Value, Error> {
    run_fn(prog, "main", out)
}

// 引数の無い関数 entry を実行する（main と hibana test のテスト）
pub fn run_fn(prog: &Program, entry: &str, out: &mut dyn Write) -> Result<Value, Error> {
    let jobs = Rc::new(Jobs::default());
    let mut it = Interp::new(prog, out, &jobs);
    let runtime = it.runtime.clone();
    let f = runtime.fns[entry];
    let result = jobs.promise(async move { it.call(f, vec![], false).await });
    finish(&jobs, &result, &runtime).map_err(|flow| {
        let (msg, span) = match flow {
            Flow::Error(msg, span) => (msg, span),
            Flow::Abort(..) => ("with で処理されなかった中断".into(), None),
            Flow::Return(_) | Flow::Break | Flow::Continue => unreachable!("関数本体の外には出ない"),
        };
        let err = Error::from(format!("実行時エラー: {msg}"));
        let err = match span { Some(span) => err.at(span), None => err };
        err.with_sources(prog.modules.iter().map(|module| &module.source))
    })
}

// マクロ本体を実行する。put はコンパイル時の出力として標準エラーに書く
pub fn run_macro(b: &Block, vars: Vec<(Name, Value)>, prog: &Program) -> Result<Value, String> {
    let mut err = std::io::stderr();
    let jobs = Rc::new(Jobs::default());
    let mut it = Interp::new(prog, &mut err, &jobs);
    let runtime = it.runtime.clone();
    let result = jobs.promise(async move { it.block(b, &mut vec![vars.into_iter().collect()]).await });
    match finish(&jobs, &result, &runtime) {
        Ok(v) | Err(Flow::Return(v)) => Ok(v),
        Err(Flow::Error(e, _)) => Err(e),
        Err(Flow::Abort(..)) => unreachable!("handle の外には出ない"),
        Err(Flow::Break | Flow::Continue) => unreachable!("ループの外には出ない"),
    }
}

fn finish(jobs: &Jobs<'_>, promise: &Promise, runtime: &Runtime<'_>) -> R<Value> {
    while jobs.step() {}
    let result = promise.result().unwrap_or_else(|| error("非同期処理が完了しなかった".into()));
    match runtime.callback_error.borrow_mut().take() {
        Some(e) if result.is_ok() => Err(e),
        _ => result,
    }
}

fn quote_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r"\\"),
            '\u{8}' => out.push_str(r"\b"),
            '\t' => out.push_str(r"\t"),
            '\n' => out.push_str(r"\n"),
            '\u{c}' => out.push_str(r"\f"),
            '\r' => out.push_str(r"\r"),
            '\0'..='\u{1f}' | '\u{7f}'..='\u{9f}' => write!(out, r"\u{:04x}", c as u32).unwrap(),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn show_number(n: f64) -> String {
    // JSON の数値変換で最少桁と等距離での偶数丸めを得て、小数点の位置を調整する。
    let Some(number) = serde_json::Number::from_f64(n) else {
        return n.to_string();
    };
    let mut text = number.to_string();
    let Some((mantissa, exponent)) = text.split_once('e') else {
        if text.ends_with(".0") {
            text.truncate(text.len() - 2);
        }
        return text;
    };

    let (sign, mantissa) = mantissa.strip_prefix('-').map_or(("", mantissa), |m| ("-", m));
    let exponent: i32 = exponent.parse().expect("JSON の指数は整数");
    let point = mantissa.find('.').unwrap_or(mantissa.len()) as i32 + exponent;
    let digits = mantissa.replace('.', "");
    let digits = digits.trim_end_matches('0');
    if point <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-point) as usize))
    } else {
        let point = point as usize;
        if point >= digits.len() {
            format!("{sign}{digits}{}", "0".repeat(point - digits.len()))
        } else {
            let (whole, fraction) = digits.split_at(point);
            format!("{sign}{whole}.{fraction}")
        }
    }
}

fn show(v: &Value) -> String {
    match v {
        Value::Arr(xs) => format!("[{}]", xs.iter().map(show).collect::<Vec<_>>().join(", ")),
        Value::Int(n) => n.to_string(),
        Value::Num(n) => show_number(*n),
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => quote_string(s),
        Value::Ty(t) => t.to_string(),
        Value::Unit => "()".into(),
        Value::Syn(s) => format!("{:?}", s.node.strip()),
        Value::Fn(_) => "<fn>".into(),
        Value::Task(_) => "<task>".into(),
        Value::Ref(_) => "<ref>".into(),
        Value::Data(d) if d.1.is_empty() => d.0.to_string(),
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
    Err(Flow::Error(msg, None))
}

fn int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        _ => unreachable!("型検査済み"),
    }
}

fn number(v: &Value) -> f64 {
    match v { Value::Int(n) => *n as f64, Value::Num(n) => *n, _ => unreachable!("型検査済み") }
}

fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Ref(a), Value::Ref(b)) => Rc::ptr_eq(a, b),
        (Value::Int(i), Value::Num(f)) | (Value::Num(f), Value::Int(i)) => (*i as f64) == *f,
        (Value::Arr(a), Value::Arr(b)) => a.len() == b.len() && a.iter().zip(b.iter()).all(|(a,b)| value_eq(a,b)),
        (Value::Data(a), Value::Data(b)) => a.0 == b.0 && a.1.len() == b.1.len() && a.1.iter().zip(&b.1).all(|(a,b)| value_eq(a,b)),
        _ => a == b,
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
        // 指数が 52 を超えると、-1・0・1 以外は範囲外になる。-1・0・1 は偶奇だけで決まる
        "**" if b > 52 && !(-1..=1).contains(&a) => None,
        "**" => a.checked_pow(if b > 52 { 52 + b % 2 } else { b } as u32),
        _ => a.checked_rem(b),
    }.filter(|n| (MIN_SAFE_INT..=MAX_SAFE_INT).contains(n))
}

// 整数の指数は各出力先で乗算の順序もそろえる。
pub(crate) fn num_pow(a: f64, b: f64) -> Option<f64> {
    if b.fract() != 0.0 { return None; }
    let mut n = b.abs();
    let mut factor = if b < 0.0 { 1.0 / a } else { a };
    let mut result = 1.0;
    while n != 0.0 {
        if n % 2.0 != 0.0 { result *= factor; }
        n = (n / 2.0).trunc();
        if n != 0.0 { factor *= factor; }
    }
    Some(result)
}

impl<'a> Interp<'a> {
    fn new(prog: &'a Program, out: &'a mut dyn Write, jobs: &Rc<Jobs<'a>>) -> Self {
        Self {
            runtime: Rc::new(Runtime {
                fns: prog.fns.iter().map(|f| (f.name.as_str(), f)).collect(),
                ctors: prog.datas.iter().flat_map(|d| &d.ctors).map(|c| {
                    (c.name.as_str(), (&c.name, c.fields.iter().map(|field| &field.0).collect()))
                }).collect(),
                out: RefCell::new(out),
                info: &prog.async_info,
                next_h: Cell::new(0),
                callback_error: RefCell::new(None),
            }),
            jobs: Rc::downgrade(jobs),
            handlers: Vec::new(),
            variant: false,
        }
    }

    fn fork(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            jobs: self.jobs.clone(),
            handlers: self.handlers.clone(),
            variant: self.variant,
        }
    }

    fn jobs(&self) -> Rc<Jobs<'a>> {
        self.jobs.upgrade().expect("実行中は仕事の並びを保持する")
    }

    fn waits(&self, id: Option<NodeId>) -> bool {
        let info = self.runtime.info;
        id.is_some_and(|id| info.awaits.contains(&id) || (self.variant && info.var_awaits.contains(&id)))
    }

    fn start_apply(&self, f: Value, args: Vec<Value>) -> Rc<Promise> {
        let mut child = self.fork();
        self.jobs().promise(async move { child.apply(&f, args).await })
    }

    async fn apply_at(&mut self, f: Value, args: Vec<Value>, id: Option<NodeId>) -> R<Value> {
        if self.waits(id) {
            self.start_apply(f, args).wait().await
        } else {
            self.apply(&f, args).await
        }
    }

    async fn all(&self, fs: &[Value]) -> R<Value> {
        let jobs = self.jobs();
        let promises: Vec<_> = fs.iter().map(|f| self.start_apply(f.clone(), vec![])).collect();
        let all = Rc::new(Promise::default());
        if promises.is_empty() {
            all.settle(Ok(Value::Arr(Rc::new(vec![]))));
        }
        let results = Rc::new(RefCell::new((promises.len(), vec![Value::Unit; promises.len()])));
        for (i, promise) in promises.into_iter().enumerate() {
            let all = all.clone();
            let results = results.clone();
            jobs.start(async move {
                match promise.wait().await {
                    Ok(value) => {
                        let (left, values) = &mut *results.borrow_mut();
                        values[i] = value;
                        *left -= 1;
                        if *left == 0 {
                            all.settle(Ok(Value::Arr(Rc::new(std::mem::take(values)))));
                        }
                    }
                    Err(e) => all.settle(Err(e)),
                }
            });
        }
        // JavaScript では all の本体と、all を呼んだ所の両方で待つ
        let result = jobs.promise(async move { all.wait().await });
        drop(jobs);
        result.wait().await
    }

    fn spawn(&self, f: Value) -> Value {
        let promise = self.start_apply(f, vec![]);
        let task = Rc::new(Task(Promise::default()));
        let done = task.clone();
        self.jobs().start(async move {
            let result = promise.wait().await;
            match &result {
                Err(Flow::Error(msg, Some(span))) => {
                    eprintln!("実行時エラー: {msg}（{} 行 {} 列）", span.start.line, span.start.column);
                }
                Err(Flow::Error(msg, None)) => eprintln!("実行時エラー: {msg}"),
                Err(Flow::Abort(..)) => eprintln!("実行時エラー: with で処理されなかった中断"),
                _ => {}
            }
            done.0.settle(result);
        });
        Value::Task(task)
    }

    fn task_then(&self, task: Rc<Task>, f: Value) {
        let mut child = self.fork();
        self.jobs().start(async move {
            if let Ok(value) = task.0.wait().await
                && let Err(e) = child.apply(&f, vec![value]).await
            {
                child.runtime.callback_error.borrow_mut().get_or_insert(e);
            }
        });
    }

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
                | "chars"
                | "code"
                | "char_from_codepoint"
                | "panic"
                | "now_ms"
                | "ident"
        ) || self.runtime.fns.contains_key(name)
        {
            return Ok(None);
        }
        if name == "now_ms" {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            return Ok(Some(Value::Int(t.as_millis() as i64)));
        }
        let a = vs.remove(0);
        match (name, &a) {
            ("error" | "panic", _) => return error(str_val(a).to_string()),
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
            ("chars", Value::Str(s)) => {
                return Ok(Some(Value::Arr(Rc::new(
                    s.chars()
                        .map(|c| Value::Str(c.to_string().into()))
                        .collect(),
                ))));
            }
            ("char_from_codepoint", Value::Int(n)) => {
                let s = u32::try_from(*n).ok().and_then(char::from_u32)
                    .map(|c| c.to_string()).unwrap_or_default();
                return Ok(Some(Value::Str(s.into())));
            }
            ("code", Value::Str(s)) => {
                return Ok(Some(Value::Int(s.chars().next().map_or(-1, |c| c as i64))));
            }
            ("ident", Value::Str(s)) => {
                let tok = Token {
                    tok: Tok::Ident(s.to_string().into()),
                    span: Span::default(),
                    site: None,
                    origin: None,
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
            ("str_of", Node::Record { .. } | Node::Type(_)) => {
                let Node::At(range, _) = &s.node else { return error("構文の元のトークン列が無い".into()); };
                let mut text = String::new();
                for t in &s.toks[range.clone()] {
                    if t.gap_before && !text.is_empty() { text.push(' '); }
                    use crate::lexer::Tok;
                    text.push_str(&match &t.tok {
                        Tok::Ident(n) => n.to_string(), Tok::Punct(n) => n.to_string(),
                        Tok::Str(n) => format!("{n:?}"), Tok::Int(n) => n.to_string(), Tok::Float(n) => n.to_string(),
                        Tok::Raw(c) => c.to_string(), Tok::Template(n, _) => format!("`{n}`"), Tok::Eof => String::new(),
                    });
                }
                Value::Str(text.into())
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
            Value::Data(d) if d.0.structure_owner().is_some() => {
                let fs = &self.runtime.ctors[d.0.as_str()].1;
                let name = d.0.structure_owner().unwrap();
                // struct Name(型, ...) は Name(1, 2)
                if d.0.structure_style() == Some(crate::frontend::name::StructureStyle::Tuple) {
                    let body: Vec<String> = d.1.iter().map(|x| self.show(x)).collect();
                    return format!("{name}({})", body.join(", "));
                }
                let body: Vec<String> = fs
                    .iter()
                    .zip(&d.1)
                    .map(|(f, x)| format!("{f}: {}", self.show(x)))
                    .collect();
                format!("{name} {{ {} }}", body.join(", "))
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
    async fn apply(&mut self, f: &Value, args: Vec<Value>) -> R<Value> {
        let Value::Fn(c) = f else {
            unreachable!("型検査済み")
        };
        match &**c {
            Callable::Named(n) => self.call(self.runtime.fns[n.as_str()], args, false).await,
            Callable::Lam { params, body, env, variant } => {
                let mut scope = env.clone();
                scope.extend(params.iter().cloned().zip(args));
                let saved = std::mem::replace(&mut self.variant, *variant);
                let result = self.block(body, &mut vec![scope]).await;
                self.variant = saved;
                match result {
                    Err(Flow::Return(v)) => Ok(v),
                    r => r,
                }
            }
        }
    }

    async fn call_at(&mut self, f: &'a Fn, args: Vec<Value>, id: Option<NodeId>) -> R<Value> {
        let info = self.runtime.info;
        let variant = id.is_some_and(|id| {
            info.async_inst.contains(&id) || (self.variant && info.var_awaits.contains(&id)
                && f.params[..f.implicit].iter().any(|(_, t)| matches!(t.peel(), Expr::Var(n) if n == "Effect")))
        });
        if variant || has_async(f) {
            let mut child = self.fork();
            let pending = self.jobs().promise(async move { child.call(f, args, variant).await });
            pending.wait().await
        } else {
            self.call(f, args, false).await
        }
    }

    async fn call(&mut self, f: &'a Fn, args: Vec<Value>, variant: bool) -> R<Value> {
        if f.op.is_some() {
            let k = args.len() - (f.params.len() - f.implicit);
            return self.perform(f, args[k..].to_vec()).await;
        }
        if f.js.is_some() {
            return error(format!(
                "extern の関数 {} は JavaScript 出力（hibana::to_js）でだけ使える", f.name
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
        let saved = std::mem::replace(&mut self.variant, variant);
        let result = self.block(&f.body, &mut env).await;
        self.variant = saved;
        match result {
            Err(Flow::Return(v)) => Ok(v),
            r => r,
        }
    }

    // 操作を、それを処理する一番内側の handle の腕で実行する。腕の中の操作は、その handle より外で処理する
    async fn perform(&mut self, op: &Fn, args: Vec<Value>) -> R<Value> {
        let name = &op.name;
        let Some(i) = self.handlers.iter().rposition(|h| h.1.contains_key(name)) else {
            return error(format!("操作 {name} を処理する handle が無い"));
        };
        let (id, f) = (self.handlers[i].0, self.handlers[i].1[name].clone());
        let saved = self.handlers.split_off(i);
        let r = if has_async(op) {
            self.start_apply(f, args).wait().await
        } else {
            Box::pin(self.apply(&f, args)).await
        };
        self.handlers.extend(saved);
        match r? {
            Value::Data(d) if d.0 == "$resume" => Ok(d.1[0].clone()),
            v => Err(Flow::Abort(id, v)),
        }
    }

    async fn block(&mut self, b: &Block, env: &mut Vec<HashMap<Name, Value>>) -> R<Value> {
        env.push(HashMap::new());
        let r = self.block_inner(b, env).await;
        env.pop();
        r
    }

    async fn for_body(
        &mut self,
        x: &Name,
        body: &Block,
        env: &mut Vec<HashMap<Name, Value>>,
        items: impl Iterator<Item = Value>,
    ) -> R<()> {
        for v in items {
            env.push(HashMap::from([(x.clone(), v)]));
            let r = self.block(body, env).await;
            env.pop();
            match r {
                Ok(_) | Err(Flow::Continue) => {}
                Err(Flow::Break) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    async fn block_inner(&mut self, b: &Block, env: &mut Vec<HashMap<Name, Value>>) -> R<Value> {
        for s in &b.stmts {
            match s {
                Stmt::Let { name, init, .. } => {
                    let v = self.expr(init, env).await?;
                    env.last_mut().unwrap().insert(name.clone(), v);
                }
                Stmt::Expr(e) => {
                    self.expr(e, env).await?;
                }
            }
        }
        match &b.tail {
            Some(e) => self.expr(e, env).await,
            None => Ok(Value::Unit),
        }
    }

    fn expr<'b>(&'b mut self, mut e: &'b Expr, env: &'b mut Vec<HashMap<Name, Value>>) -> impl Future<Output = R<Value>> + 'b {
        let (mut id, mut span) = (None, None);
        loop {
            e = match e {
                Expr::Id(i, inner) => { id = Some(*i); inner }
                Expr::At(at, inner) => { span = Some(*at); inner }
                Expr::InModule(_, inner) => inner,
                _ => break,
            };
        }
        // 再帰で通る式を分け、呼び出しのたびに大きな評価用のスタック区画を作らない
        let work: Pin<Box<dyn Future<Output = R<Value>> + 'b>> = match e {
            Expr::Call(name, args) => Box::pin(self.call_expr(name, args, env, id)),
            Expr::Binary(op, l, r) => Box::pin(self.binary(op, l, r, env)),
            Expr::If(c, then, els) => Box::pin(self.if_expr(c, then, els.as_deref(), env)),
            _ => Box::pin(self.expr_inner(e, env, id)),
        };
        async move {
            work.await.map_err(|flow| match flow {
                Flow::Error(msg, None) => Flow::Error(msg, span),
                _ => flow,
            })
        }
    }

    async fn call_expr(&mut self, name: &Name, args: &[Expr], env: &mut Vec<HashMap<Name, Value>>, id: Option<NodeId>) -> R<Value> {
        Ok({
            let mut vs = Vec::new();
            for a in args {
                vs.push(self.expr(a, env).await?);
            }
            if name == "#num" {
                match vs.remove(0) { Value::Int(n) => Value::Num(n as f64), v @ Value::Num(_) => v, _ => unreachable!("型検査済み") }
            } else if name == "num_floor" || name == "num_sqrt" {
                let n = number(&vs[0]);
                if name == "num_floor" {
                    let n = n.floor();
                    if !(MIN_SAFE_INT as f64..=MAX_SAFE_INT as f64).contains(&n) { return error("floor の結果が int の範囲外".into()); }
                    Value::Int(n as i64)
                } else {
                    if n < 0.0 { return error("sqrt の引数が負".into()); }
                    Value::Num(n.sqrt())
                }
            } else if let Some(f) = env.iter().rev().find_map(|s| s.get(name)).cloned() {
                self.apply_at(f, vs, id).await?
            } else if name == "all" && !self.runtime.fns.contains_key(name.as_str()) {
                let Value::Arr(fs) = &vs[0] else {
                    unreachable!("型検査済み")
                };
                self.all(fs).await?
            } else if name == "spawn" && !self.runtime.fns.contains_key(name.as_str()) {
                self.spawn(vs.remove(0))
            } else if matches!(name.as_str(), "task_then" | "task_done" | "task_value")
                && !self.runtime.fns.contains_key(name.as_str())
            {
                let Value::Task(task) = vs.remove(0) else { unreachable!("型検査済み") };
                match name.as_str() {
                    "task_then" => {
                        self.task_then(task, vs.remove(0));
                        Value::Unit
                    }
                    "task_done" => Value::Bool(matches!(task.0.result(), Some(Ok(_)))),
                    "task_value" => task.0.result().and_then(Result::ok).unwrap_or(Value::Unit),
                    _ => unreachable!("型検査済み"),
                }
            } else if name == "new_ref" && !self.runtime.fns.contains_key(name.as_str()) {
                Value::Ref(Rc::new(std::cell::RefCell::new(vs.pop().unwrap())))
            } else if name == "read" && !self.runtime.fns.contains_key(name.as_str()) {
                let Value::Ref(r) = &vs[0] else {
                    unreachable!("型検査済み")
                };
                r.borrow().clone()
            } else if name == "write" && !self.runtime.fns.contains_key(name.as_str()) {
                let x = vs.pop().unwrap();
                let Value::Ref(r) = &vs[0] else {
                    unreachable!("型検査済み")
                };
                *r.borrow_mut() = x;
                Value::Unit
            } else if name == "push" && !self.runtime.fns.contains_key(name.as_str()) {
                let x = vs.pop().unwrap();
                let Value::Arr(xs) = vs.pop().unwrap() else {
                    unreachable!("型検査済み")
                };
                let mut xs = xs.to_vec();
                xs.push(x);
                Value::Arr(Rc::new(xs))
            } else if name == "set_at" && !self.runtime.fns.contains_key(name.as_str()) {
                let x = vs.pop().unwrap();
                let i = int(vs.pop().unwrap());
                let Value::Arr(xs) = vs.pop().unwrap() else {
                    unreachable!("型検査済み")
                };
                let mut xs = xs.to_vec();
                xs[i as usize] = x;
                Value::Arr(Rc::new(xs))
            } else if name == "#resume" {
                // 腕の値として返し、perform が呼んだ所の値にする
                Value::Data(Rc::new((
                    "$resume".into(),
                    vec![vs.pop().unwrap_or(Value::Unit)],
                )))
            } else if self.runtime.ctors.contains_key(name.as_str()) {
                Value::Data(Rc::new((self.runtime.ctors[name.as_str()].0.clone(), vs)))
            } else if !self.runtime.fns.contains_key(name.as_str())
                && name != "put"
                && name.starts_with(|c: char| c.is_ascii_uppercase())
            {
                // 型を作る呼び出し（Option(int) など）。実行時には名前だけを持つ
                Value::Ty(name.as_str().into())
            } else if name == "input" {
                self.runtime.out.borrow_mut().flush().map_err(|e| Flow::Error(e.to_string(), None))?;
                let mut line = String::new();
                std::io::stdin().read_line(&mut line).map_err(|e| Flow::Error(format!("標準入力を読めません: {e}"), None))?;
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') { line.pop(); }
                }
                Value::Str(line.into())
            } else if let Some(v) = self.syn_fn(name, &mut vs)? {
                v
            } else if name == "put" {
                let s = match &vs[0] {
                    Value::Str(s) => s.to_string(),
                    v => self.show(v),
                };
                writeln!(self.runtime.out.borrow_mut(), "{s}").map_err(|e| Flow::Error(e.to_string(), None))?;
                Value::Unit
            } else {
                self.call_at(self.runtime.fns[name.as_str()], vs, id).await?
            }
        })
    }

    async fn binary(&mut self, op: &str, l: &Expr, r: &Expr, env: &mut Vec<HashMap<Name, Value>>) -> R<Value> {
        Ok(match op {
            "&&" => {
                Value::Bool(boolean(self.expr(l, env).await?) && boolean(self.expr(r, env).await?))
            }
            "||" => {
                Value::Bool(boolean(self.expr(l, env).await?) || boolean(self.expr(r, env).await?))
            }
            _ => {
                let (a, b) = (self.expr(l, env).await?, self.expr(r, env).await?);
                if matches!(a, Value::Num(_)) || matches!(b, Value::Num(_)) {
                    let (x, y) = (number(&a), number(&b));
                    return match op {
                        "==" => Ok(Value::Bool(x == y)), "!=" => Ok(Value::Bool(x != y)),
                        "<" => Ok(Value::Bool(x < y)), "<=" => Ok(Value::Bool(x <= y)), ">" => Ok(Value::Bool(x > y)), ">=" => Ok(Value::Bool(x >= y)),
                        "/" | "%" if y == 0.0 => error("0 で除算した".into()),
                        op => {
                            let n = match op {
                                "+" => x + y,
                                "-" => x - y,
                                "*" => x * y,
                                "/" => x / y,
                                "%" => x % y,
                                "**" => num_pow(x, y).ok_or_else(|| Flow::Error("** の指数は整数値に限る".into(), None))?,
                                _ => unreachable!("型検査済み"),
                            };
                            if !n.is_finite() { return error("number の演算結果が有限でない".into()); }
                            Ok(Value::Num(n))
                        }
                    };
                }
                match op {
                    "+" if matches!(a, Value::Str(_)) => {
                        Value::Str(format!("{}{}", str_val(a), str_val(b)).into())
                    }
                    "==" => Value::Bool(value_eq(&a, &b)),
                    "!=" => Value::Bool(!value_eq(&a, &b)),
                    "<" => Value::Bool(int(a) < int(b)),
                    "<=" => Value::Bool(int(a) <= int(b)),
                    ">" => Value::Bool(int(a) > int(b)),
                    ">=" => Value::Bool(int(a) >= int(b)),
                    _ => {
                        let (a, b) = (int(a), int(b));
                        if b == 0 && matches!(op, "/" | "%") {
                            return error("0 で除算した".into());
                        }
                        if b < 0 && op == "**" {
                            return error("負の指数".into());
                        }
                        Value::Int(arith(op, a, b).ok_or_else(|| Flow::Error("オーバーフロー".into(), None))?)
                    }
                }
            }
        })
    }

    async fn if_expr(&mut self, c: &Expr, then: &Block, els: Option<&Expr>, env: &mut Vec<HashMap<Name, Value>>) -> R<Value> {
        if boolean(self.expr(c, env).await?) {
            self.block(then, env).await
        } else if let Some(x) = els {
            self.expr(x, env).await
        } else {
            Ok(Value::Unit)
        }
    }

    async fn expr_inner(&mut self, e: &Expr, env: &mut Vec<HashMap<Name, Value>>, id: Option<NodeId>) -> R<Value> {
        Ok(match e {
            Expr::Id(..) | Expr::InModule(..) | Expr::At(..) |
            Expr::Call(..) | Expr::Binary(..) | Expr::If(..) => unreachable!(),
            Expr::Int(n) => Value::Int(*n),
            Expr::Num(n) => Value::Num(*n),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Str(s) => Value::Str(s.as_str().into()),
            Expr::Method(..) | Expr::Macro(..) | Expr::Await(_) | Expr::Try(_) => unreachable!("型検査で弾く・書き換える"),
            // 細分化型は実行時には名前だけを持つ型
            Expr::Refine(..) => Value::Ty("refine".into()),
            Expr::Array(items) => {
                let mut vs = Vec::new();
                for a in items {
                    vs.push(self.expr(a, env).await?);
                }
                Value::Arr(Rc::new(vs))
            }
            Expr::Index(v, i) => {
                let Value::Arr(xs) = self.expr(v, env).await? else {
                    unreachable!("型検査済み")
                };
                let i = int(self.expr(i, env).await?);
                // 範囲は型検査で証明済み
                xs[i as usize].clone()
            }
            Expr::Field(x, name) => {
                let v = self.expr(x, env).await?;
                if let Value::Data(d) = &v {
                    let i = self.runtime.ctors[d.0.as_str()].1
                        .iter()
                        .position(|field| *field == name)
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
                let mut flat: Vec<(Name, Value)> = env
                    .iter()
                    .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect();
                let mut out = Vec::new();
                let mut ev = |e: &Expr, vars: &[(Name, Value)]| {
                    env.push(vars.iter().cloned().collect());
                    // 差し込む式はその場で値にする
                    let r = match std::pin::pin!(self.expr(e, env)).poll(&mut Context::from_waker(Waker::noop())) {
                        Poll::Ready(result) => result,
                        Poll::Pending => error("マクロの差し込みの途中で非同期の処理を待てない".into()),
                    };
                    env.pop();
                    r.map_err(|f| match f {
                        Flow::Error(e, _) => Error::from(e),
                        Flow::Return(_) => "${ } の中で return した".into(),
                        Flow::Abort(..) => unreachable!("マクロ本体に handle は無い"),
                        Flow::Break | Flow::Continue => unreachable!("ループの外には出ない"),
                    })
                };
                meta::splice(q, &mut flat, &mut out, &mut ev).map_err(|e| Flow::Error(e.msg, None))?;
                let n = out.len();
                Value::Syn(Rc::new(Syn {
                    node: Node::At(0..n, Box::new(Node::Raw)),
                    toks: Rc::new(out),
                }))
            }
            Expr::Var(n) => match env.iter().rev().find_map(|s| s.get(n)) {
                Some(v) => v.clone(),
                None if self.runtime
                    .fns
                    .get(n.as_str())
                    .is_some_and(|f| f.alias && f.params.is_empty()) =>
                {
                    self.call(self.runtime.fns[n.as_str()], vec![], false).await?
                }
                None if self.runtime.fns.contains_key(n.as_str()) => {
                    Value::Fn(Rc::new(Callable::Named(n.clone())))
                }
                None if self.runtime.ctors.contains_key(n.as_str()) => {
                    Value::Data(Rc::new((self.runtime.ctors[n.as_str()].0.clone(), vec![])))
                }
                // 型検査を通っているので、残りは int などの型名
                None => Value::Ty(n.as_str().into()),
            },
            Expr::Unary("-", x) => {
                let value = self.expr(x, env).await?;
                if let Value::Num(n) = value { return Ok(Value::Num(-n)); }
                let n = int(value);
                Value::Int(
                    n.checked_neg().filter(|n| (MIN_SAFE_INT..=MAX_SAFE_INT).contains(n))
                        .ok_or_else(|| Flow::Error("オーバーフロー".into(), None))?,
                )
            }
            Expr::Unary(_, x) => Value::Bool(!boolean(self.expr(x, env).await?)),
            Expr::Lam(params, _, _, body) => Value::Fn(Rc::new(Callable::Lam {
                params: params.iter().map(|p| p.0.clone()).collect(),
                body: body.clone(),
                env: env
                    .iter()
                    .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect(),
                variant: self.variant,
            })),
            Expr::Pi(..) => Value::Ty("fn".into()),
            Expr::Handle(node, body, arms) => {
                let id = self.runtime.next_h.get() + 1;
                self.runtime.next_h.set(id);
                let mut captured: HashMap<Name, Value> = env
                    .iter()
                    .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect();
                // 腕の中の exit は、この with の番号へ抜ける
                captured.insert(exit_name(*node), Value::Int(id as i64));
                let arms = arms
                    .iter()
                    .map(|a| {
                        let body = Block { id: Default::default(),
                            stmts: vec![],
                            tail: Some(Box::new(a.body.clone())),
                        };
                        let f = Callable::Lam {
                            params: a.binds.clone(),
                            body,
                            env: captured.clone(),
                            variant: self.variant,
                        };
                        (a.ctor.clone(), Value::Fn(Rc::new(f)))
                    })
                    .collect();
                let n = self.handlers.len();
                self.handlers.push((id, Rc::new(arms)));
                let r = self.expr(body, env).await;
                self.handlers.truncate(n);
                match r {
                    Err(Flow::Abort(h, v)) if h == id => v,
                    r => r?,
                }
            }
            Expr::Exit(target, x) => {
                let v = self.expr(x, env).await?;
                let name = exit_name(target.expect("型検査が行き先を書き込む"));
                let Some(Value::Int(h)) = env.iter().rev().find_map(|s| s.get(&name)) else {
                    unreachable!("exit は with の腕の中だけ")
                };
                return Err(Flow::Abort(*h as usize, v));
            }
            Expr::Apply(f, args) => {
                let f = self.expr(f, env).await?;
                let mut vs = Vec::new();
                for a in args {
                    vs.push(self.expr(a, env).await?);
                }
                self.apply_at(f, vs, id).await?
            }
            Expr::Match(s, arms) => {
                let Value::Data(d) = self.expr(s, env).await? else {
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
                let r = self.expr(&arm.body, env).await;
                env.pop();
                r?
            }
            // x = push(x, e): 他から指されていなければ（Rc が一つなら）写さずに足す
            Expr::Assign(name, x)
                if x.self_push(name).is_some() && !self.runtime.fns.contains_key("push") =>
            {
                let v = self.expr(x.self_push(name).unwrap(), env).await?;
                let slot = env.iter_mut().rev().find_map(|s| s.get_mut(name)).unwrap();
                let Value::Arr(xs) = slot else {
                    unreachable!("型検査済み")
                };
                Rc::make_mut(xs).push(v);
                Value::Unit
            }
            Expr::Assign(name, x)
                if x.self_set(name).is_some() && !self.runtime.fns.contains_key("set_at") =>
            {
                let (i, e) = x.self_set(name).unwrap();
                let i = int(self.expr(i, env).await?);
                let v = self.expr(e, env).await?;
                let slot = env.iter_mut().rev().find_map(|s| s.get_mut(name)).unwrap();
                let Value::Arr(xs) = slot else {
                    unreachable!("型検査済み")
                };
                Rc::make_mut(xs)[i as usize] = v;
                Value::Unit
            }
            Expr::Assign(name, x) => {
                let v = self.expr(x, env).await?;
                *env.iter_mut().rev().find_map(|s| s.get_mut(name)).unwrap() = v;
                Value::Unit
            }
            Expr::For(x, it, end, body) => {
                let it = self.expr(it, env).await?;
                match end {
                    Some(end) => {
                        let range = int(it)..int(self.expr(end, env).await?);
                        self.for_body(x, body, env, range.map(Value::Int)).await?;
                    }
                    None => match it {
                        Value::Arr(xs) => self.for_body(x, body, env, xs.iter().cloned()).await?,
                        _ => unreachable!("型検査済み"),
                    },
                }
                Value::Unit
            }
            Expr::While(c, body) => {
                while boolean(self.expr(c, env).await?) {
                    match self.block(body, env).await {
                        Ok(_) | Err(Flow::Continue) => {}
                        Err(Flow::Break) => break,
                        Err(e) => return Err(e),
                    }
                }
                Value::Unit
            }
            Expr::Break => return Err(Flow::Break),
            Expr::Continue => return Err(Flow::Continue),
            Expr::Block(b) => self.block(b, env).await?,
            Expr::Return(x) => {
                let v = match x {
                    Some(x) => self.expr(x, env).await?,
                    None => Value::Unit,
                };
                return Err(Flow::Return(v));
            }
        })
    }
}

fn has_async(f: &Fn) -> bool {
    f.effects.iter().any(|e| matches!(e.peel(), Expr::Var(n) if n == "Async"))
}

// with（Handle）の番号を腕に渡す変数の名前
fn exit_name(node: crate::parser::NodeId) -> Name {
    Name::temporary("exit", node.raw() as usize)
}
