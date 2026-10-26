// 依存型の型検査。型は式であり、検査中に評価（正規化）して比べる
use crate::meta::{Kinds, Q};
use crate::parser::{Arm, Block, Ctor, Data, Expr, Fn, Program, Stmt};
use crate::smt::Smt;
use std::collections::HashMap;
use std::rc::Rc;

use crate::error::{Ctx, Error, R};

// 型検査中の値。型そのものも値として表す
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum V {
    Int(i64),
    Bool(bool),
    Str(Rc<str>),
    Unit,
    Arr(Rc<Vec<V>>),
    // 型の階層 Type(n)。Type は Type(0) で、Type(n) : Type(n + 1)
    Univ(u32),
    // data の値 (構築子, フィールド)
    Data(Rc<(String, Vec<V>)>),
    // data の型 (型名, 引数)
    DataT(Rc<(String, Vec<V>)>),
    // int・bool・str・syn・unit・never。never は return の型で、どの型とも一致する
    Prim(&'static str),
    // Vec<要素の型>
    VecT(Rc<V>),
    // 細分化型 base[x -> 条件]。条件は x を Var(0) として評価した値
    Refine(Rc<(V, V)>),
    // 関数型 (x: A, ...) -> B
    Pi(Rc<PiV>),
    // 無名関数の値
    Lam(Rc<Clo>),
    // トップレベルの関数を値として使ったもの
    FnRef(String),
    // 種類の分かっているコード片 syn<種類>。種類の分からないものは Prim("syn")
    Syn(Rc<str>),
    // 値が分からず計算が止まった項
    Neu(Rc<Neu>),
}

// 関数型の値。引数 i は Var(binder(level, i)) として doms と cod の中に現れる
#[derive(Debug)]
pub(crate) struct PiV {
    doms: Vec<V>,
    cod: V,
    level: usize,
}

// 深さ level は比べない。依存しない関数型は、どの深さで書いても同じ型になる
impl PartialEq for PiV {
    fn eq(&self, o: &Self) -> bool {
        self.doms == o.doms && self.cod == o.cod
    }
}

// 無名関数。作った時の変数の値を持つ
#[derive(Debug)]
pub(crate) struct Clo {
    params: Vec<String>,
    body: Block,
    env: HashMap<String, V>,
}

// ponytail: 無名関数は同じ値の時だけ等しいとみなす。中身を比べるなら本体を正規化して比べる
impl PartialEq for Clo {
    fn eq(&self, o: &Self) -> bool {
        std::ptr::eq(self, o)
    }
}

// 関数型の引数を表す変数の番号。入れ子の深さ level ごとに分け、内側の関数型の引数と混ざらないようにする
// ponytail: 深さの違う所で書いた同じ依存関数型は別の型になる。α同値で比べるなら de Bruijn 添字にする
fn binder(level: usize, i: usize) -> usize {
    (1 << 40) + level * 64 + i
}

#[derive(Debug, PartialEq)]
pub(crate) enum Neu {
    // 引数など、値の分からない変数。番号で区別する。0 は細分化型の条件の変数
    Var(usize, String),
    Un(&'static str, V),
    Bin(&'static str, V, V),
    // 展開しきれなかった関数呼び出し
    Call(String, Vec<V>),
    // 値の分からない関数の呼び出し
    App(V, Vec<V>),
    // 値の分からない構造体のフィールド
    Proj(V, String),
    // 型の計算で扱えない式（while・代入・print など）。どの値とも等しくない
    Opaque(usize),
}

// js は JavaScript の値をそのまま持ち回る型（extern で使う）
const PRIMS: &[&str] = &["int", "bool", "str", "syn", "unit", "js"];

// マクロ本体の中でだけ使える組み込み関数 (名前, 引数, 戻り値)
const SYN_FNS: &[(&str, &[&str], &str)] = &[
    ("at", &["syn", "int"], "syn"),
    ("is_null", &["syn"], "bool"),
    ("int_of", &["syn"], "int"),
    ("str_of", &["syn"], "str"),
    ("error", &["str"], "never"),
    // 文字列から識別子のコード片を作る。呼び出し側から見える名前を導入する時に使う
    ("ident", &["str"], "syn"),
];

// どこでも使える組み込み関数
const BUILTINS: &[&str] = &[
    "print",
    "len",
    "Vec",
    "Type",
    "to_str",
    "split",
    "starts_with",
    "slice",
];

// 文字列の組み込み関数 (名前, 引数, 戻り値)。split の戻り値は Vec<str>
const STR_FNS: &[(&str, &[&str], &str)] = &[
    ("split", &["str", "str"], "Vec<str>"),
    ("starts_with", &["str", "str"], "bool"),
    ("slice", &["str", "int", "int"], "str"),
];

fn prim(name: &str) -> V {
    V::Prim(
        PRIMS
            .iter()
            .chain(&["never"])
            .find(|p| **p == name)
            .unwrap(),
    )
}

fn syn_of(kind: &str) -> V {
    if kind.is_empty() {
        prim("syn")
    } else {
        V::Syn(kind.into())
    }
}

fn neu(n: Neu) -> V {
    V::Neu(Rc::new(n))
}

// 細分化型の条件の中の変数
fn bound() -> V {
    neu(Neu::Var(0, "_".into()))
}

fn show(v: &V) -> String {
    match v {
        V::Int(n) => n.to_string(),
        V::Bool(b) => b.to_string(),
        V::Str(s) => format!("{s:?}"),
        V::Unit => "()".into(),
        V::Arr(v) => format!("[{}]", v.iter().map(show).collect::<Vec<_>>().join(", ")),
        V::Univ(0) => "Type".into(),
        V::Univ(n) => format!("Type<{n}>"),
        V::DataT(d) if !d.1.is_empty() => format!(
            "{}<{}>",
            d.0,
            d.1.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        V::Data(d) | V::DataT(d) if d.1.is_empty() => d.0.clone(),
        V::Data(d) | V::DataT(d) => format!(
            "{}({})",
            d.0,
            d.1.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        V::Prim(p) => p.to_string(),
        V::VecT(a) => format!("Vec<{}>", show(a)),
        V::Refine(r) => format!("{}[_ -> {}]", show(&r.0), show(&r.1)),
        V::Pi(p) => format!(
            "({}) -> {}",
            p.doms.iter().map(show).collect::<Vec<_>>().join(", "),
            show(&p.cod)
        ),
        V::Lam(_) => "<fn>".into(),
        V::Syn(k) => format!("syn<{k}>"),
        V::FnRef(n) => n.clone(),
        V::Neu(n) => match &**n {
            Neu::Var(_, name) => name.clone(),
            Neu::Un(op, x) => format!("{op}{}", show(x)),
            Neu::Bin(op, a, b) => format!("({} {op} {})", show(a), show(b)),
            Neu::Call(f, args) if f == "[]" => format!("{}[{}]", show(&args[0]), show(&args[1])),
            // 型の別名は大文字で始まる名前とみなして <> で表す
            Neu::Call(f, args) if f.starts_with(|c: char| c.is_ascii_uppercase()) => {
                format!(
                    "{f}<{}>",
                    args.iter().map(show).collect::<Vec<_>>().join(", ")
                )
            }
            Neu::Call(f, args) => format!(
                "{f}({})",
                args.iter().map(show).collect::<Vec<_>>().join(", ")
            ),
            Neu::App(f, args) => format!(
                "{}({})",
                show(f),
                args.iter().map(show).collect::<Vec<_>>().join(", ")
            ),
            Neu::Proj(x, f) => format!("{}.{f}", show(x)),
            Neu::Opaque(_) => "?".into(),
        },
    }
}

// 細分化を外した型と、その条件の列
fn split(v: &V) -> (V, Vec<V>) {
    let mut preds = Vec::new();
    let mut v = v.clone();
    while let V::Refine(r) = &v {
        preds.push(r.1.clone());
        v = r.0.clone();
    }
    (v, preds)
}

fn base(v: &V) -> V {
    split(v).0
}

// 比較を a <= b + k の形に直す。a < b は a <= b - 1、b が定数なら a <= 0 + k
fn le_form(v: &V) -> Option<(V, V, i64)> {
    let V::Neu(n) = v else { return None };
    let (a, b, k): (&V, &V, i64) = match &**n {
        Neu::Bin("<=", a, b) => (a, b, 0),
        Neu::Bin("<", a, b) => (a, b, -1),
        _ => return None,
    };
    let (b, kb) = match b {
        V::Int(c) => (V::Int(0), *c),
        V::Neu(m) => match &**m {
            Neu::Bin("+", x, V::Int(c)) => (x.clone(), *c),
            Neu::Bin("-", x, V::Int(c)) => (x.clone(), c.saturating_neg()),
            _ => (b.clone(), 0),
        },
        _ => (b.clone(), 0),
    };
    Some((a.clone(), b, k.saturating_add(kb)))
}

// 変数 id が値の中に現れるか
fn mentions(v: &V, id: usize) -> bool {
    match v {
        V::Data(d) | V::DataT(d) => d.1.iter().any(|x| mentions(x, id)),
        V::Arr(xs) => xs.iter().any(|x| mentions(x, id)),
        V::VecT(a) => mentions(a, id),
        V::Refine(r) => mentions(&r.0, id) || mentions(&r.1, id),
        V::Pi(p) => p.doms.iter().any(|x| mentions(x, id)) || mentions(&p.cod, id),
        V::Lam(c) => c.env.values().any(|x| mentions(x, id)),
        V::Neu(n) => match &**n {
            Neu::Var(i, _) => *i == id,
            Neu::Un(_, x) => mentions(x, id),
            Neu::Bin(_, a, b) => mentions(a, id) || mentions(b, id),
            Neu::App(f, args) => mentions(f, id) || args.iter().any(|a| mentions(a, id)),
            Neu::Proj(x, _) => mentions(x, id),
            Neu::Call(_, args) => args.iter().any(|a| mentions(a, id)),
            Neu::Opaque(_) => false,
        },
        _ => false,
    }
}

// 評価が止まる理由。Stuck は値の分からない条件で分岐した時など
enum Halt {
    Stuck,
    Err(Error),
}

type H<T> = Result<T, Halt>;

// 型の計算をする評価器
// ponytail: 停止性を検査しない。燃料が尽きたらエラーにする
struct Norm<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    datas: HashMap<&'a str, &'a Data>,
    // 構築子名から (型, 構築子)
    ctors: HashMap<&'a str, (&'a Data, &'a Ctor)>,
    fuel: usize,
    // 展開の入れ子の深さ。スタックあふれを防ぐ
    depth: usize,
    // 関数型の入れ子の深さ。関数型の引数の変数の番号に使う
    pi_depth: usize,
    next_id: usize,
}

type Env = Vec<HashMap<String, V>>;

impl Norm<'_> {
    fn new() -> Self {
        Norm {
            fns: HashMap::new(),
            datas: HashMap::new(),
            ctors: HashMap::new(),
            fuel: 100_000,
            depth: 0,
            pi_depth: 0,
            next_id: 0,
        }
    }

    fn fresh(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    fn opaque(&mut self) -> V {
        let id = self.fresh();
        neu(Neu::Opaque(id))
    }

    // 止まった計算は、どの値とも等しくない項にする
    fn eval_or_opaque(&mut self, e: &Expr, env: &mut Env) -> R<V> {
        match self.eval(e, env) {
            Ok(v) => Ok(v),
            Err(Halt::Stuck) => Ok(self.opaque()),
            Err(Halt::Err(e)) => Err(e),
        }
    }

    fn un(op: &'static str, v: V) -> V {
        if let V::Neu(n) = &v
            && let (true, Neu::Bin(o, a, b)) = (op == "!", &**n)
        {
            // 否定は比較を反転させて、証明で照合しやすい形にする
            let flip = match *o {
                "<" => Some(("<=", b, a)),
                "<=" => Some(("<", b, a)),
                "==" => Some(("!=", a, b)),
                "!=" => Some(("==", a, b)),
                _ => None,
            };
            if let Some((o, a, b)) = flip {
                return Self::bin(o, a.clone(), b.clone());
            }
        }
        match (op, v) {
            ("-", V::Int(n)) if n != i64::MIN => V::Int(-n),
            ("!", V::Bool(b)) => V::Bool(!b),
            (op, v) => neu(Neu::Un(op, v)),
        }
    }

    fn bin(op: &'static str, a: V, b: V) -> V {
        use V::*;
        let r = match (op, &a, &b) {
            ("+", Int(x), Int(y)) => x.checked_add(*y).map(Int),
            ("+", Str(x), Str(y)) => Some(Str(format!("{x}{y}").into())),
            ("-", Int(x), Int(y)) => x.checked_sub(*y).map(Int),
            ("*", Int(x), Int(y)) => x.checked_mul(*y).map(Int),
            ("/", Int(x), Int(y)) => x.checked_div(*y).map(Int),
            ("%", Int(x), Int(y)) => x.checked_rem(*y).map(Int),
            ("<", Int(x), Int(y)) => Some(Bool(x < y)),
            ("<=", Int(x), Int(y)) => Some(Bool(x <= y)),
            (">", Int(x), Int(y)) => Some(Bool(x > y)),
            (">=", Int(x), Int(y)) => Some(Bool(x >= y)),
            ("&&", Bool(x), Bool(y)) => Some(Bool(*x && *y)),
            ("||", Bool(x), Bool(y)) => Some(Bool(*x || *y)),
            // 同じ項は値が分からなくても等しい
            ("==", _, _) if a == b => Some(Bool(true)),
            ("!=", _, _) if a == b => Some(Bool(false)),
            ("==" | "!=", x, y) if !matches!(x, Neu(_)) && !matches!(y, Neu(_)) => {
                Some(Bool((a == b) == (op == "==")))
            }
            _ => None,
        };
        if let Some(r) = r {
            return r;
        }
        // > と >= は向きを揃えて < と <= にする
        match op {
            ">" => neu(self::Neu::Bin("<", b, a)),
            ">=" => neu(self::Neu::Bin("<=", b, a)),
            _ => neu(self::Neu::Bin(op, a, b)),
        }
    }

    fn len(v: V) -> V {
        match &v {
            V::Arr(xs) => V::Int(xs.len() as i64),
            V::Str(s) => V::Int(s.chars().count() as i64),
            _ => neu(Neu::Call("len".into(), vec![v])),
        }
    }

    // 構造体のフィールド。値が分からなければ x.f という項にする
    fn proj(&self, x: V, f: &str) -> V {
        if let V::Data(d) = &x
            && let Some((_, c)) = self.ctors.get(d.0.as_str())
            && let Some(i) = c.fields.iter().position(|(n, _)| n == f)
        {
            return d.1[i].clone();
        }
        neu(Neu::Proj(x, f.to_string()))
    }

    // 関数の値を呼ぶ
    fn app(&mut self, f: V, args: Vec<V>) -> R<V> {
        let V::Lam(c) = &f else {
            return match &f {
                V::FnRef(n) => self.call(n, args),
                _ => Ok(neu(Neu::App(f.clone(), args))),
            };
        };
        if self.fuel == 0 || self.depth >= 64 {
            return Err("型の計算が終わらない（無名関数の展開）".into());
        }
        self.fuel -= 1;
        let mut scope = c.env.clone();
        scope.extend(c.params.iter().cloned().zip(args.iter().cloned()));
        let mut env = vec![scope];
        self.depth += 1;
        let r = self.block(&c.body, &mut env);
        self.depth -= 1;
        match r {
            Ok(v) => Ok(v),
            Err(Halt::Stuck) => Ok(neu(Neu::App(f.clone(), args))),
            Err(Halt::Err(e)) => Err(e),
        }
    }

    // 関数型の式を評価する。引数は Var(binder(深さ, i)) にする
    fn eval_pi(&mut self, binders: &[(String, Expr)], ret: &Expr, env: &mut Env) -> H<V> {
        let level = self.pi_depth;
        self.pi_depth += 1;
        env.push(HashMap::new());
        let r = (|| {
            let mut doms = Vec::new();
            for (i, (n, t)) in binders.iter().enumerate() {
                doms.push(self.eval(t, env)?);
                env.last_mut()
                    .unwrap()
                    .insert(n.clone(), neu(Neu::Var(binder(level, i), n.clone())));
            }
            let cod = self.eval(ret, env)?;
            Ok(V::Pi(Rc::new(PiV { doms, cod, level })))
        })();
        env.pop();
        self.pi_depth -= 1;
        r
    }

    // 関数を展開する。中で止まったら呼び出しのまま残す
    fn call(&mut self, name: &str, args: Vec<V>) -> R<V> {
        match (name, args.as_slice()) {
            ("len", [v]) => return Ok(Self::len(v.clone())),
            ("Vec", [a]) => return Ok(V::VecT(Rc::new(a.clone()))),
            ("Type", [V::Int(n)]) if *n >= 0 => return Ok(V::Univ(*n as u32)),
            _ if self.ctors.contains_key(name) => return Ok(V::Data(Rc::new((name.into(), args)))),
            _ if self.datas.contains_key(name) => {
                return Ok(V::DataT(Rc::new((name.into(), args))));
            }
            _ => {}
        }
        // partial fn は止まるとは限らないので展開しない
        let Some(f) = self.fns.get(name).copied().filter(|f| !f.partial) else {
            return Ok(neu(Neu::Call(name.into(), args)));
        };
        // 省かれたジェネリック引数は、値の計算には使わないので () で埋める
        let args = if args.len() + f.implicit == f.params.len() {
            let mut full = vec![V::Unit; f.implicit];
            full.extend(args);
            full
        } else {
            args
        };
        if self.fuel == 0 || self.depth >= 64 {
            return Err(format!("型の計算が終わらない（{name} の展開）").into());
        }
        self.fuel -= 1;
        let mut env = vec![
            f.params
                .iter()
                .map(|(n, _)| n.clone())
                .zip(args.iter().cloned())
                .collect(),
        ];
        self.depth += 1;
        let r = self.block(&f.body, &mut env);
        self.depth -= 1;
        match r {
            Ok(v) => Ok(v),
            Err(Halt::Stuck) => Ok(neu(Neu::Call(name.into(), args))),
            Err(Halt::Err(e)) => Err(e),
        }
    }

    fn block(&mut self, b: &Block, env: &mut Env) -> H<V> {
        env.push(HashMap::new());
        let r = self.block_inner(b, env);
        env.pop();
        r
    }

    fn block_inner(&mut self, b: &Block, env: &mut Env) -> H<V> {
        for s in &b.stmts {
            match s {
                Stmt::Let { mutable: true, .. } => return Err(Halt::Stuck),
                Stmt::Let { name, init, .. } => {
                    let v = self.eval(init, env)?;
                    env.last_mut().unwrap().insert(name.clone(), v);
                }
                Stmt::Expr(e) => {
                    self.eval(e, env)?;
                }
            }
        }
        match &b.tail {
            Some(e) => self.eval(e, env),
            None => Ok(V::Unit),
        }
    }

    fn eval(&mut self, e: &Expr, env: &mut Env) -> H<V> {
        Ok(match e {
            Expr::At(_, x) => self.eval(x, env)?,
            Expr::Int(n) => V::Int(*n),
            Expr::Bool(b) => V::Bool(*b),
            Expr::Str(s) => V::Str(s.as_str().into()),
            Expr::Var(n) => match env.iter().rev().find_map(|s| s.get(n)) {
                Some(v) => v.clone(),
                None if n == "Type" => V::Univ(0),
                None if PRIMS.contains(&n.as_str()) => prim(n),
                None if self
                    .fns
                    .get(n.as_str())
                    .is_some_and(|f| f.alias && f.params.is_empty()) =>
                {
                    self.call(n, vec![]).map_err(Halt::Err)?
                }
                None if self.ctors.contains_key(n.as_str())
                    || self.datas.contains_key(n.as_str()) =>
                {
                    self.call(n, vec![]).map_err(Halt::Err)?
                }
                None if self.fns.contains_key(n.as_str()) => V::FnRef(n.clone()),
                None => return Err(Halt::Err(format!("未定義の変数 {n}").into())),
            },
            Expr::Unary(op, x) => Self::un(op, self.eval(x, env)?),
            Expr::Binary(op, l, r) => {
                let a = self.eval(l, env)?;
                // 左辺で決まる && と || は右辺を評価しない
                match (*op, &a) {
                    ("&&", V::Bool(false)) | ("||", V::Bool(true)) => return Ok(a),
                    _ => {}
                }
                let b = self.eval(r, env)?;
                Self::bin(op, a, b)
            }
            Expr::Call(name, _) if name == "print" || SYN_FNS.iter().any(|s| s.0 == name) => {
                return Err(Halt::Stuck);
            }
            Expr::Call(name, args) => {
                let mut vs = Vec::new();
                for a in args {
                    vs.push(self.eval(a, env)?);
                }
                match env.iter().rev().find_map(|s| s.get(name)).cloned() {
                    Some(f) => self.app(f, vs).map_err(Halt::Err)?,
                    None => self.call(name, vs).map_err(Halt::Err)?,
                }
            }
            Expr::Apply(f, args) => {
                let f = self.eval(f, env)?;
                let mut vs = Vec::new();
                for a in args {
                    vs.push(self.eval(a, env)?);
                }
                self.app(f, vs).map_err(Halt::Err)?
            }
            Expr::Lam(params, _, body) => V::Lam(Rc::new(Clo {
                params: params.iter().map(|p| p.0.clone()).collect(),
                body: body.clone(),
                env: env
                    .iter()
                    .flat_map(|s| s.iter().map(|(k, v)| (k.clone(), v.clone())))
                    .collect(),
            })),
            Expr::Pi(binders, ret) => self.eval_pi(binders, ret, env)?,
            Expr::Array(items) => {
                let mut vs = Vec::new();
                for a in items {
                    vs.push(self.eval(a, env)?);
                }
                V::Arr(Rc::new(vs))
            }
            Expr::Field(x, f) if !matches!(self.eval(x, env)?, V::Prim("syn") | V::Syn(_)) => {
                let v = self.eval(x, env)?;
                if !matches!(v, V::Data(_) | V::Neu(_)) {
                    return Err(Halt::Stuck);
                }
                self.proj(v, f)
            }
            Expr::Match(s, arms) => {
                let V::Data(d) = self.eval(s, env)? else {
                    return Err(Halt::Stuck);
                };
                let arm = arms
                    .iter()
                    .find(|a| a.ctor == d.0 || a.ctor == "_")
                    .ok_or(Halt::Stuck)?;
                let mut scope = HashMap::new();
                if arm.ctor != "_" {
                    scope.extend(arm.binds.iter().cloned().zip(d.1.iter().cloned()));
                }
                env.push(scope);
                let r = self.eval(&arm.body, env);
                env.pop();
                r?
            }
            Expr::Index(v, i) => match (self.eval(v, env)?, self.eval(i, env)?) {
                (V::Arr(xs), V::Int(i)) if usize::try_from(i).is_ok_and(|i| i < xs.len()) => {
                    xs[i as usize].clone()
                }
                // 値の分からない配列や添字は v[i] という項にする（範囲は型検査で証明済み）
                (v @ V::Neu(_), i) | (v, i @ V::Neu(_)) => neu(Neu::Call("[]".into(), vec![v, i])),
                _ => return Err(Halt::Stuck),
            },
            Expr::Refine(b, x, p) => {
                let b = self.eval(b, env)?;
                env.push(HashMap::from([(x.clone(), bound())]));
                let p = self.eval_or_opaque(p, env);
                env.pop();
                V::Refine(Rc::new((b, p.map_err(Halt::Err)?)))
            }
            Expr::If(c, then, els) => match self.eval(c, env)? {
                V::Bool(true) => self.block(then, env)?,
                V::Bool(false) => match els {
                    Some(x) => self.eval(x, env)?,
                    None => V::Unit,
                },
                _ => return Err(Halt::Stuck),
            },
            Expr::Block(b) => self.block(b, env)?,
            Expr::Assign(..)
            | Expr::For(..)
            | Expr::While(..)
            | Expr::Return(_)
            | Expr::Field(..)
            | Expr::Method(..)
            | Expr::Quote(_)
            | Expr::Macro(..) => return Err(Halt::Stuck),
        })
    }

    // 変数 id を by に置き換え、止まっていた計算を進め直す
    fn subst(&mut self, v: &V, id: usize, by: &V) -> R<V> {
        Ok(match v {
            V::Arr(xs) => V::Arr(Rc::new(
                xs.iter().map(|x| self.subst(x, id, by)).collect::<R<_>>()?,
            )),
            V::VecT(a) => V::VecT(Rc::new(self.subst(a, id, by)?)),
            V::Data(d) => V::Data(Rc::new((
                d.0.clone(),
                d.1.iter()
                    .map(|x| self.subst(x, id, by))
                    .collect::<R<_>>()?,
            ))),
            V::DataT(d) => V::DataT(Rc::new((
                d.0.clone(),
                d.1.iter()
                    .map(|x| self.subst(x, id, by))
                    .collect::<R<_>>()?,
            ))),
            V::Refine(r) => V::Refine(Rc::new((
                self.subst(&r.0, id, by)?,
                self.subst(&r.1, id, by)?,
            ))),
            V::Pi(p) => V::Pi(Rc::new(PiV {
                doms: p
                    .doms
                    .iter()
                    .map(|x| self.subst(x, id, by))
                    .collect::<R<_>>()?,
                cod: self.subst(&p.cod, id, by)?,
                level: p.level,
            })),
            V::Lam(c) if c.env.values().any(|x| mentions(x, id)) => {
                let mut env = HashMap::new();
                for (k, x) in &c.env {
                    env.insert(k.clone(), self.subst(x, id, by)?);
                }
                V::Lam(Rc::new(Clo {
                    params: c.params.clone(),
                    body: c.body.clone(),
                    env,
                }))
            }
            V::Neu(n) => match &**n {
                Neu::Var(i, _) if *i == id => by.clone(),
                Neu::Var(..) | Neu::Opaque(_) => v.clone(),
                Neu::Un(op, x) => Self::un(op, self.subst(x, id, by)?),
                Neu::Bin(op, a, b) => {
                    let (a, b) = (self.subst(a, id, by)?, self.subst(b, id, by)?);
                    Self::bin(op, a, b)
                }
                Neu::Call(f, args) => {
                    let args = args
                        .iter()
                        .map(|a| self.subst(a, id, by))
                        .collect::<R<_>>()?;
                    self.call(f, args)?
                }
                Neu::Proj(x, f) => {
                    let x = self.subst(x, id, by)?;
                    self.proj(x, f)
                }
                Neu::App(f, args) => {
                    let f = self.subst(f, id, by)?;
                    let args = args
                        .iter()
                        .map(|a| self.subst(a, id, by))
                        .collect::<R<_>>()?;
                    self.app(f, args)?
                }
            },
            v => v.clone(),
        })
    }
}

// match の場合に入った時の (添字の照合で得た代入, フィールドの値)
type Entered = (Vec<(usize, V)>, Vec<V>);

// 変数の (型, 値, 可変か)
struct Entry {
    ty: V,
    val: V,
    mutable: bool,
}

// 変数と、そこで成り立つと分かっている事実（bool の値）
#[derive(Default)]
struct Scope {
    vars: HashMap<String, Entry>,
    facts: Vec<V>,
}

// ponytail: エラーに行番号が無い。AST にスパンを持たせる時に追加する
struct Ck<'a> {
    norm: Norm<'a>,
    scopes: Vec<Scope>,
    ret: V,
    // マクロ本体（コンパイル時に実行するコード）の中か
    in_macro: bool,
    smt: Smt,
    // data の型の階層
    levels: HashMap<String, u32>,
    // 停止性の検査。検査中の関数、その引数の (最初の値, 型)、どの引数が再帰呼び出しで減り続けているか
    cur_fn: Option<String>,
    cur_params: Vec<(V, V)>,
    decr: Vec<bool>,
    // match で取り出したフィールドの変数から、取り出し元の変数へ
    sub: HashMap<usize, usize>,
    // マクロ本体の検査で使う、syntax 規則ごとのフィールドの種類
    rules: HashMap<String, Kinds>,
}

// never はどの型とも一致する。細分化された値は元の型として使える
fn same(a: &V, b: &V) -> bool {
    let never = V::Prim("never");
    if a == b || *a == never || *b == never {
        return true;
    }
    // Vec は書き換えられないので、要素の細分化を外して使ってよい
    match (a, &base(b)) {
        (V::VecT(x), V::VecT(y)) => same(x, y),
        // 種類の分からない syn とは互いに使える
        (V::Prim("syn"), V::Syn(_)) | (V::Syn(_), V::Prim("syn")) => true,
        // 下の階層の型は上の階層の型としても使える
        (V::Univ(i), V::Univ(j)) => j <= i,
        (a, b) => a == b,
    }
}

fn want(e: &V, got: &V, what: &str) -> R<()> {
    if same(e, got) {
        Ok(())
    } else {
        Err(format!("{what}: {} が必要だが {}", show(e), show(got)).into())
    }
}

// 式とその部分式をすべて f に渡す
fn walk(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    let block = |b: &Block, f: &mut dyn FnMut(&Expr)| {
        for s in &b.stmts {
            match s {
                Stmt::Let { init, .. } => walk(init, f),
                Stmt::Expr(e) => walk(e, f),
            }
        }
        if let Some(t) = &b.tail {
            walk(t, f);
        }
    };
    match e {
        Expr::At(_, x)
        | Expr::Unary(_, x)
        | Expr::Field(x, _)
        | Expr::Assign(_, x)
        | Expr::Return(Some(x)) => walk(x, f),
        Expr::Binary(_, a, b) | Expr::Index(a, b) | Expr::Refine(a, _, b) => {
            walk(a, f);
            walk(b, f);
        }
        Expr::Call(_, xs) | Expr::Array(xs) => xs.iter().for_each(|x| walk(x, f)),
        Expr::Method(x, _, xs) => {
            walk(x, f);
            xs.iter().for_each(|x| walk(x, f));
        }
        Expr::If(c, t, e) => {
            walk(c, f);
            block(t, f);
            if let Some(e) = e {
                walk(e, f);
            }
        }
        Expr::While(c, b) => {
            walk(c, f);
            block(b, f);
        }
        Expr::Block(b) => block(b, f),
        Expr::Match(s, arms) => {
            walk(s, f);
            arms.iter().for_each(|a| walk(&a.body, f));
        }
        Expr::Lam(_, _, b) => block(b, f),
        Expr::For(_, a, b, body) => {
            walk(a, f);
            if let Some(b) = b {
                walk(b, f);
            }
            block(body, f);
        }
        Expr::Pi(bs, r) => {
            bs.iter().for_each(|(_, t)| walk(t, f));
            walk(r, f);
        }
        Expr::Apply(x, xs) => {
            walk(x, f);
            xs.iter().for_each(|x| walk(x, f));
        }
        _ => {}
    }
}

// 式の中で代入される変数名を集める
fn assigned(e: &Expr, out: &mut Vec<String>) {
    walk(e, &mut |x| {
        if let Expr::Assign(n, _) = x {
            out.push(n.clone());
        }
    });
}

// 型 name が関数型の引数の側に現れるか（正値性の検査）
fn negative(v: &V, name: &str) -> bool {
    match v {
        V::Pi(p) => p.doms.iter().any(|x| occurs(x, name)) || negative(&p.cod, name),
        V::DataT(d) => d.1.iter().any(|x| negative(x, name)),
        V::VecT(a) => negative(a, name),
        V::Refine(r) => negative(&r.0, name),
        _ => false,
    }
}

fn occurs(v: &V, name: &str) -> bool {
    match v {
        V::DataT(d) => d.0 == name || d.1.iter().any(|x| occurs(x, name)),
        V::Pi(p) => p.doms.iter().any(|x| occurs(x, name)) || occurs(&p.cod, name),
        V::VecT(a) => occurs(a, name),
        V::Refine(r) => occurs(&r.0, name),
        _ => false,
    }
}

// quote の中の $x.y の先頭の名前を集める
fn q_names(q: &[Q], out: &mut Vec<String>) {
    for x in q {
        match x {
            Q::Var { path, .. } => out.push(path[0].clone()),
            Q::Rep(inner, _) => q_names(inner, out),
            _ => {}
        }
    }
}

// 呼び出しの関係を辿って from から to に着くか
fn reaches(graph: &HashMap<&str, Vec<String>>, from: &str, to: &str) -> bool {
    let mut seen = vec![from.to_string()];
    let mut todo = vec![from.to_string()];
    while let Some(f) = todo.pop() {
        for g in graph.get(f.as_str()).into_iter().flatten() {
            if g == to {
                return true;
            }
            if !seen.contains(g) {
                seen.push(g.clone());
                todo.push(g.clone());
            }
        }
    }
    false
}

// 関数の本体から呼ぶ関数名を集める
fn callees(f: &Fn) -> Vec<String> {
    let mut out = Vec::new();
    walk(&Expr::Block(f.body.clone()), &mut |x| {
        if let Expr::Call(n, _) | Expr::Var(n) = x {
            out.push(n.clone());
        }
    });
    out
}

pub fn check(prog: &Program) -> R<()> {
    let fns = &prog.fns;
    let mut ck = Ck {
        norm: Norm::new(),
        scopes: Vec::new(),
        ret: V::Unit,
        in_macro: false,
        smt: Smt::new(),
        levels: HashMap::new(),
        cur_fn: None,
        cur_params: Vec::new(),
        decr: Vec::new(),
        sub: HashMap::new(),
        rules: HashMap::new(),
    };
    // 関数名・型名・構築子名は同じ名前空間にある
    let mut names: Vec<&str> = Vec::new();
    for n in prog
        .datas
        .iter()
        .flat_map(|d| std::iter::once(&d.name).chain(d.ctors.iter().map(|c| &c.name)))
        .chain(fns.iter().map(|f| &f.name))
    {
        let reserved = BUILTINS.contains(&n.as_str())
            || PRIMS.contains(&n.as_str())
            || SYN_FNS.iter().any(|s| s.0 == n);
        if reserved || names.contains(&n.as_str()) {
            return Err(format!("名前 {n} が重複している").into());
        }
        names.push(n);
    }
    for d in &prog.datas {
        ck.norm.datas.insert(&d.name, d);
        for c in &d.ctors {
            ck.norm.ctors.insert(&c.name, (d, c));
        }
        ck.levels.insert(d.name.clone(), 0);
    }
    for f in fns {
        ck.norm.fns.insert(&f.name, f);
    }
    // 相互再帰は減る引数の対応を追えないので、partial fn にしてもらう
    let graph: HashMap<&str, Vec<String>> =
        fns.iter().map(|f| (f.name.as_str(), callees(f))).collect();
    for f in fns.iter().filter(|f| !f.partial) {
        for g in graph[f.name.as_str()]
            .iter()
            .filter(|g| **g != f.name && graph.contains_key(g.as_str()))
        {
            if reaches(&graph, g, &f.name) {
                return Err(format!("関数 {} と {g} が互いに呼び合っている。停止性を検査できないので partial fn にする", f.name).into());
            }
        }
    }
    // data は自分自身を含みうるので、階層が変わらなくなるまで計算し直す
    for _ in 0..8 {
        let mut changed = false;
        for d in &prog.datas {
            let l = ck.data_level(d)?;
            changed |= ck.levels.insert(d.name.clone(), l) != Some(l);
        }
        if !changed {
            break;
        }
    }
    match ck.norm.fns.get("main") {
        Some(f) if f.params.is_empty() => {}
        _ => return Err("引数なしの main 関数が必要".into()),
    }
    for f in fns {
        let r = |e: &str| format!("関数 {}: {e}", f.name);
        ck.scopes = vec![Scope::default()];
        ck.cur_fn = None;
        for (n, t) in &f.params {
            let ty = ck.eval_type(t).ctx(|e| r(&format!("引数 {n}: {e}")))?;
            let id = ck.norm.fresh();
            ck.bind(n, ty, neu(Neu::Var(id, n.clone())), false).ctx(r)?;
        }
        ck.ret = match &f.ret {
            Some(t) => ck.eval_type(t).ctx(|e| r(&format!("戻り値: {e}")))?,
            None => prim("unit"),
        };
        if !f.alias && matches!(base(&ck.ret), V::Univ(_)) {
            return Err(r("関数は型を返せない。型を計算するなら type で定義する").into());
        }
        if f.partial && ck.is_proof_type(&ck.ret) {
            return Err(r("partial fn は証明の型（添字付きの型族）を返せない。止まらない関数で偽の証明が作れてしまうため").into());
        }
        // extern の関数は本体が無いので、型の宣言だけを検査する
        if f.js.is_some() {
            continue;
        }
        ck.cur_fn = (!f.partial).then(|| f.name.clone());
        ck.cur_params = f
            .params
            .iter()
            .map(|(n, _)| {
                let e = ck.lookup(n).unwrap();
                (e.val.clone(), e.ty.clone())
            })
            .collect();
        ck.decr = vec![true; f.params.len()];
        let ret = ck.ret.clone();
        // 戻り値が unit の関数は最後の式の値を捨てる
        if ret == prim("unit") {
            ck.block(&f.body).ctx(r)?;
        } else {
            ck.check_block(&f.body, &ret)
                .ctx(|e| r(&format!("戻り値: {e}")))?;
        }
    }
    Ok(())
}

// マクロ本体を、束縛名を syn 型の変数として検査する。値は syn 型でなければならない
pub fn check_macro(b: &Block, params: &Kinds, rules: &HashMap<String, Kinds>) -> R<()> {
    let mut ck = Ck {
        norm: Norm::new(),
        scopes: vec![Scope::default()],
        ret: prim("syn"),
        in_macro: true,
        smt: Smt::new(),
        levels: HashMap::new(),
        cur_fn: None,
        cur_params: Vec::new(),
        decr: Vec::new(),
        sub: HashMap::new(),
        rules: rules.clone(),
    };
    for (p, k) in params {
        let v = ck.norm.opaque();
        ck.bind(p, syn_of(k), v, false)?;
    }
    let t = ck.block(b)?;
    want(&prim("syn"), &t, "マクロ本体の値")
}

impl Ck<'_> {
    fn push(&mut self) {
        self.scopes.push(Scope::default());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    // 構造体の値なら、フィールドの型の細分化の条件を v.f についての事実として加える
    fn struct_facts(&mut self, ty: &V, val: &V) -> R<()> {
        let V::DataT(d) = base(ty) else { return Ok(()) };
        let data = self.norm.datas[d.0.as_str()];
        let [c] = data.ctors.as_slice() else {
            return Ok(());
        };
        if !c.name.ends_with("{}") {
            return Ok(());
        }
        let mut env: Env = vec![
            data.params
                .iter()
                .map(|p| p.0.clone())
                .zip(d.1.iter().cloned())
                .collect(),
        ];
        for (n, ft) in &c.fields {
            let t = self.norm.eval_or_opaque(ft, &mut env)?;
            let fv = self.norm.proj(val.clone(), n);
            for p in split(&t).1 {
                let f = self.norm.subst(&p, 0, &fv)?;
                self.add_fact(f);
            }
            env[0].insert(n.clone(), fv);
        }
        Ok(())
    }

    // 変数を束縛し、型の細分化の条件をその変数についての事実として加える
    fn bind(&mut self, name: &str, ty: V, val: V, mutable: bool) -> R<()> {
        for p in split(&ty).1 {
            let f = self.norm.subst(&p, 0, &val)?;
            self.add_fact(f);
        }
        self.struct_facts(&ty, &val)?;
        self.scopes
            .last_mut()
            .unwrap()
            .vars
            .insert(name.into(), Entry { ty, val, mutable });
        Ok(())
    }

    fn lookup(&self, name: &str) -> Option<&Entry> {
        self.scopes.iter().rev().find_map(|s| s.vars.get(name))
    }

    fn add_fact(&mut self, v: V) {
        if let V::Neu(n) = &v
            && let Neu::Bin("&&", a, b) = &**n
        {
            self.add_fact(a.clone());
            self.add_fact(b.clone());
        } else if v != V::Bool(true) {
            self.scopes.last_mut().unwrap().facts.push(v);
        }
    }

    // 変数の値が変わるので、その変数についての事実を捨てる。型の条件は代入でも保たれるので残す
    fn forget(&mut self, name: &str) -> R<()> {
        let Some(en) = self.lookup(name) else {
            return Ok(());
        };
        let (ty, val) = (en.ty.clone(), en.val.clone());
        let V::Neu(n) = &val else { return Ok(()) };
        let Neu::Var(id, _) = &**n else { return Ok(()) };
        for s in &mut self.scopes {
            s.facts.retain(|f| !mentions(f, *id));
        }
        for p in split(&ty).1 {
            let f = self.norm.subst(&p, 0, &val)?;
            self.add_fact(f);
        }
        Ok(())
    }

    // まず事実との字面の照合と a <= b + k の弱めで示し、だめなら Z3 に問い合わせる
    fn prove(&mut self, goal: &V) -> bool {
        if self.prove_facts(goal) {
            return true;
        }
        let facts: Vec<V> = self
            .scopes
            .iter()
            .flat_map(|s| s.facts.iter().cloned())
            .collect();
        self.smt.prove(&facts, goal)
    }

    fn prove_facts(&self, goal: &V) -> bool {
        if let V::Neu(n) = goal {
            match &**n {
                Neu::Bin("&&", a, b) => return self.prove_facts(a) && self.prove_facts(b),
                Neu::Bin("||", a, b) => return self.prove_facts(a) || self.prove_facts(b),
                _ => {}
            }
        }
        let g_le = le_form(goal);
        let implies = |f: &V| {
            if f == goal {
                return true;
            }
            // a <= b + k は、k の大きい方へ弱められる
            if let (Some((a, b, kf)), Some((c, d, kg))) = (le_form(f), &g_le)
                && a == *c
                && b == *d
                && *kg >= kf
            {
                return true;
            }
            let (V::Neu(f), V::Neu(g)) = (f, goal) else {
                return false;
            };
            match (&**f, &**g) {
                (Neu::Bin("<", a, b), Neu::Bin("!=", c, d)) => {
                    (a == c && b == d) || (a == d && b == c)
                }
                _ => false,
            }
        };
        *goal == V::Bool(true) || self.scopes.iter().flat_map(|s| &s.facts).any(implies)
    }

    // got 型の式 e を expected 型として使えるか。expected の細分化の条件は証明する
    fn subsume(&mut self, e: &Expr, got: &V, expected: &V) -> R<()> {
        let (eb, eps) = split(expected);
        let (gb, gps) = split(got);
        if !same(&eb, &gb) {
            return Err(format!("{} が必要だが {}", show(expected), show(got)).into());
        }
        if eps.is_empty() || gb == prim("never") {
            return Ok(());
        }
        let v = self.eval_value(e)?;
        self.push();
        let mut r = Ok(());
        for gp in &gps {
            let f = self.norm.subst(gp, 0, &v)?;
            self.add_fact(f);
        }
        for ep in eps.iter().filter(|p| !gps.contains(p)) {
            let goal = self.norm.subst(ep, 0, &v)?;
            if !self.prove(&goal) {
                r = Err(format!("条件 {} を証明できない", show(&goal)).into());
                break;
            }
        }
        self.pop();
        r
    }

    // 評価器に渡す、変数の値の環境
    // ponytail: 評価のたびに全スコープを複製する。遅ければ永続的なマップにする
    fn val_env(&self) -> Env {
        vec![
            self.scopes
                .iter()
                .flat_map(|s| s.vars.iter().map(|(k, e)| (k.clone(), e.val.clone())))
                .collect(),
        ]
    }

    fn eval_value(&mut self, e: &Expr) -> R<V> {
        let mut env = self.val_env();
        self.norm.eval_or_opaque(e, &mut env)
    }

    // 式が型であることを確かめてから、その型を計算する
    fn eval_type(&mut self, e: &Expr) -> R<V> {
        self.type_level(e)?;
        self.eval_value(e)
    }

    fn block(&mut self, b: &Block) -> R<V> {
        self.push();
        let r = self.block_inner(b, None);
        self.pop();
        r
    }

    fn check_block(&mut self, b: &Block, expected: &V) -> R<()> {
        self.push();
        let r = self.block_inner(b, Some(expected));
        self.pop();
        r.map(|_| ())
    }

    // expected があれば最後の式をその型で検査する
    fn block_inner(&mut self, b: &Block, expected: Option<&V>) -> R<V> {
        let mut diverges = false;
        for s in &b.stmts {
            match s {
                Stmt::Let {
                    name,
                    mutable,
                    ty,
                    init,
                } => {
                    let t = match ty {
                        Some(ty) => {
                            let t = self.eval_type(ty)?;
                            self.check(init, &t).ctx(|e| format!("変数 {name}: {e}"))?;
                            t
                        }
                        None => self.expr(init)?,
                    };
                    // 可変な変数は、値の代わりに変数そのものを表す項を持つ
                    let val = if *mutable {
                        let id = self.norm.fresh();
                        neu(Neu::Var(id, name.clone()))
                    } else {
                        self.eval_value(init)?
                    };
                    self.bind(name, t, val, *mutable)?;
                }
                Stmt::Expr(e) => diverges |= self.expr(e)? == prim("never"),
            }
        }
        let t = match (&b.tail, expected) {
            (Some(e), Some(t)) => {
                self.check(e, t)?;
                t.clone()
            }
            (Some(e), None) => self.expr(e)?,
            (None, _) if diverges => prim("never"),
            (None, _) => prim("unit"),
        };
        if b.tail.is_none()
            && let Some(e) = expected
        {
            want(e, &t, "ブロックの値")?;
        }
        Ok(t)
    }

    // 分岐の中に入る。条件 c が b であるという事実を加える
    fn enter_branch(&mut self, c: &V, b: bool) {
        self.push();
        self.add_fact(if b {
            c.clone()
        } else {
            Norm::un("!", c.clone())
        });
    }

    // 期待する型に対して検査する。if は条件の値で期待する型を絞り込む
    fn check(&mut self, e: &Expr, expected: &V) -> R<()> {
        if let Expr::At(sp, inner) = e {
            return self.check(inner, expected).map_err(|err| err.at(*sp));
        }
        match e {
            Expr::If(c, then, Some(els)) => {
                want(&prim("bool"), &self.expr(c)?, "if の条件")?;
                let cv = self.eval_value(c)?;
                // 条件が値の分からない変数なら、各分岐ではその値が分かっているものとして扱う
                let var = match (c.peel(), &cv) {
                    (Expr::Var(n), V::Neu(nu)) => match &**nu {
                        Neu::Var(id, _) => {
                            Some((n.clone(), *id, self.lookup(n).unwrap().ty.clone()))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                for (b, branch) in [(true, None), (false, Some(&**els))] {
                    let mut exp = expected.clone();
                    self.enter_branch(&cv, b);
                    if let Some((n, id, ty)) = &var {
                        exp = self.norm.subst(expected, *id, &V::Bool(b))?;
                        self.bind(n, ty.clone(), V::Bool(b), false)?;
                    }
                    let r = match branch {
                        None => self.check_block(then, &exp),
                        Some(x) => self.check(x, &exp),
                    };
                    self.pop();
                    r?;
                }
                Ok(())
            }
            Expr::Block(b) => self.check_block(b, expected),
            Expr::Match(s, arms) => self.check_match(s, arms, Some(expected)).map(|_| ()),
            Expr::Call(n, args)
                if self.lookup(n).is_none()
                    && self
                        .norm
                        .fns
                        .get(n.as_str())
                        .is_some_and(|f| f.implicit > 0) =>
            {
                let t = self.call_fn(n, args, Some(expected))?;
                self.subsume(e, &t, expected)
            }
            Expr::Lam(params, ret, body) if matches!(base(expected), V::Pi(_)) => {
                let V::Pi(p) = base(expected) else {
                    unreachable!()
                };
                self.lam_check(params, ret, body, &p)
            }
            Expr::Call(n, args) if self.norm.ctors.contains_key(n.as_str()) => {
                let t = self.ctor_call(n, args, Some(expected))?;
                self.subsume(e, &t, expected)
            }
            Expr::Var(n)
                if self.lookup(n).is_none() && self.norm.ctors.contains_key(n.as_str()) =>
            {
                let t = self.ctor_call(n, &[], Some(expected))?;
                self.subsume(e, &t, expected)
            }
            Expr::Array(items) if matches!(base(expected), V::VecT(_)) => {
                let eb = base(expected);
                let V::VecT(a) = &eb else { unreachable!() };
                for it in items {
                    self.check(it, a)?;
                }
                self.subsume(e, &eb, expected)
            }
            e => {
                let t = self.expr(e)?;
                self.subsume(e, &t, expected)
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> R<V> {
        if let Expr::At(sp, inner) = e {
            return self.expr(inner).map_err(|err| err.at(*sp));
        }
        let int = prim("int");
        let boolean = prim("bool");
        Ok(match e {
            Expr::At(..) => unreachable!("先頭で外している"),
            Expr::Int(_) => int,
            Expr::Bool(_) => boolean,
            Expr::Str(_) => prim("str"),
            Expr::Var(n) => match self.lookup(n) {
                Some(en) => en.ty.clone(),
                None if n == "Type" => V::Univ(1),
                None if PRIMS.contains(&n.as_str()) => V::Univ(0),
                None if self
                    .norm
                    .fns
                    .get(n.as_str())
                    .is_some_and(|f| f.alias && f.params.is_empty()) =>
                {
                    return self.expr(&Expr::Call(n.clone(), vec![]));
                }
                None if self.norm.ctors.contains_key(n.as_str()) => {
                    return self.ctor_call(n, &[], None);
                }
                None if self.norm.datas.contains_key(n.as_str()) => return self.data_app(n, &[]),
                None if self.norm.fns.contains_key(n.as_str()) => {
                    let f = self.norm.fns[n.as_str()];
                    return self.fn_type(f);
                }
                None => return Err(format!("未定義の変数 {n}").into()),
            },
            Expr::Field(x, f) => {
                let t = self.expr(x)?;
                if let V::DataT(d) = base(&t) {
                    return self.struct_field(&d, x, f);
                }
                want(&prim("syn"), &t, "フィールド参照")?;
                self.syn_field(&base(&t), f)?
            }
            Expr::Method(..) => return Err("メソッドは未対応".into()),
            Expr::Macro(n, _) => return Err(format!("マクロ {n} に展開コードが無い").into()),
            Expr::Quote(q) => {
                if !self.in_macro {
                    return Err("quote はマクロ本体の中でだけ使える".into());
                }
                self.check_q(q)?;
                prim("syn")
            }
            Expr::Array(items) => {
                let Some(first) = items.first() else {
                    return Err("空の配列には型を書く（例: let v: Vec<int> = []）".into());
                };
                let a = base(&self.expr(first)?);
                for it in &items[1..] {
                    self.check(it, &a)?;
                }
                V::VecT(Rc::new(a))
            }
            Expr::Index(v, i) => {
                let V::VecT(a) = base(&self.expr(v)?) else {
                    return Err("添字は Vec にだけ使える".into());
                };
                self.check(i, &int)?;
                let (vv, iv) = (self.eval_value(v)?, self.eval_value(i)?);
                let len = Norm::len(vv);
                let goal = Norm::bin(
                    "&&",
                    Norm::bin("<=", V::Int(0), iv.clone()),
                    Norm::bin("<", iv, len),
                );
                if !self.prove(&goal) {
                    return Err(format!("添字の範囲 {} を証明できない", show(&goal)).into());
                }
                (*a).clone()
            }
            Expr::Refine(b, x, p) => {
                let level = self.type_level(b)?;
                let bt = self.eval_value(b)?;
                self.push();
                self.bind(x, bt, bound(), false)?;
                let r = self
                    .expr(p)
                    .and_then(|t| want(&boolean, &t, "細分化型の条件"));
                self.pop();
                r?;
                V::Univ(level)
            }
            Expr::Unary(op, x) => {
                let t = if *op == "-" { int } else { boolean };
                want(&t, &self.expr(x)?, op)?;
                t
            }
            Expr::Binary(op, l, r) => {
                let lt = self.expr(l)?;
                // a && b の b では a が真、a || b の b では a が偽と分かっている
                let rt = if matches!(*op, "&&" | "||") {
                    let lv = self.eval_value(l)?;
                    self.enter_branch(&lv, *op == "&&");
                    let rt = self.expr(r);
                    self.pop();
                    rt?
                } else {
                    self.expr(r)?
                };
                match *op {
                    "+" if base(&lt) == prim("str") => {
                        want(&prim("str"), &rt, op)?;
                        prim("str")
                    }
                    "+" | "-" | "*" | "/" | "%" | "<" | "<=" | ">" | ">=" => {
                        want(&int, &lt, op)?;
                        want(&int, &rt, op)?;
                        if matches!(*op, "+" | "-" | "*" | "/" | "%") {
                            int
                        } else {
                            boolean
                        }
                    }
                    "&&" | "||" => {
                        want(&boolean, &lt, op)?;
                        want(&boolean, &rt, op)?;
                        boolean
                    }
                    _ => {
                        // == と !=
                        let lt = base(&lt);
                        want(&lt, &base(&rt), op)?;
                        if lt == prim("syn") {
                            return Err("syn は比較できない".into());
                        }
                        boolean
                    }
                }
            }
            Expr::Call(name, args) if self.lookup(name).is_some() => {
                let t = self.lookup(name).unwrap().ty.clone();
                return self.apply_type(&t, args);
            }
            Expr::Call(name, args) if name == "print" => {
                if args.len() != 1 {
                    return Err("print の引数は1個".into());
                }
                self.expr(&args[0])?;
                prim("unit")
            }
            Expr::Call(name, args) if name == "Type" => match args.as_slice() {
                [a] => match a.peel() {
                    Expr::Int(n) if *n >= 0 => V::Univ(*n as u32 + 1),
                    _ => return Err("Type の引数は 0 以上の整数の定数".into()),
                },
                _ => return Err("Type の引数は1個".into()),
            },
            Expr::Call(name, args) if self.norm.ctors.contains_key(name.as_str()) => {
                return self.ctor_call(name, args, None);
            }
            Expr::Call(name, args) if self.norm.datas.contains_key(name.as_str()) => {
                return self.data_app(name, args);
            }
            Expr::Apply(f, args) => {
                let t = self.expr(f)?;
                return self.apply_type(&t, args);
            }
            Expr::Lam(params, ret, body) => return self.lam_infer(params, ret, body),
            Expr::Pi(binders, ret) => return self.pi_type(binders, ret),
            Expr::Match(s, arms) => return self.check_match(s, arms, None),
            Expr::Call(name, args) if name == "len" || name == "Vec" => {
                if args.len() != 1 {
                    return Err(format!("{name} の引数は1個").into());
                }
                if name == "Vec" {
                    let k = self
                        .type_level(&args[0])
                        .ctx(|e| format!("Vec の引数: {e}"))?;
                    return Ok(V::Univ(k));
                }
                match base(&self.expr(&args[0])?) {
                    V::VecT(_) | V::Prim("str") => int,
                    t if self.in_macro && same(&prim("syn"), &t) => int,
                    t => return Err(format!("len は Vec に使う（実際は {}）", show(&t)).into()),
                }
            }
            Expr::Call(name, args) if self.in_macro && name == "at" && args.len() == 2 => {
                let t = self.expr(&args[0])?;
                want(&prim("syn"), &t, "at")?;
                self.check(&args[1], &int)?;
                match base(&t) {
                    V::Syn(k) if k.starts_with('[') => syn_of(&k[1..k.len() - 1]),
                    V::Syn(k) => return Err(format!("at は配列に使う（実際は syn<{k}>）").into()),
                    _ => prim("syn"),
                }
            }
            Expr::Call(name, args) if self.in_macro && SYN_FNS.iter().any(|s| s.0 == name) => {
                let (_, params, ret) = SYN_FNS.iter().find(|s| s.0 == name).unwrap();
                if params.len() != args.len() {
                    return Err(format!("{name} の引数は{}個", params.len()).into());
                }
                for (p, a) in params.iter().zip(args) {
                    self.check(a, &prim(p))?;
                }
                prim(ret)
            }
            Expr::Call(name, args) if name != "to_str" && !STR_FNS.iter().any(|s| s.0 == name) => {
                if !self.norm.fns.contains_key(name.as_str()) {
                    return Err(format!("未定義の関数 {name}").into());
                }
                return self.call_fn(name, args, None);
            }
            Expr::Assign(name, x) => {
                let en = self.lookup(name).ok_or(format!("未定義の変数 {name}"))?;
                if !en.mutable {
                    return Err(format!("変数 {name} は mut でない").into());
                }
                let t = en.ty.clone();
                self.check(x, &t).ctx(|e| format!("変数 {name}: {e}"))?;
                self.forget(name)?;
                prim("unit")
            }
            Expr::If(c, then, els) => {
                want(&boolean, &self.expr(c)?, "if の条件")?;
                let cv = self.eval_value(c)?;
                self.enter_branch(&cv, true);
                let tt = self.block(then);
                self.pop();
                let tt = tt?;
                let Some(els) = els else {
                    // else が無い if の値は捨てる
                    return Ok(prim("unit"));
                };
                self.enter_branch(&cv, false);
                let et = self.expr(els);
                self.pop();
                let et = et?;
                if tt == et {
                    tt
                } else {
                    let (tb, eb) = (base(&tt), base(&et));
                    want(&tb, &eb, "if の分岐")?;
                    if tb == prim("never") { eb } else { tb }
                }
            }
            Expr::Call(name, args) if name == "to_str" => {
                let [a] = args.as_slice() else {
                    return Err("to_str の引数は1個".into());
                };
                self.expr(a)?;
                prim("str")
            }
            // 残りは文字列の組み込み関数
            Expr::Call(name, args) => {
                let (_, params, ret) = STR_FNS.iter().find(|s| s.0 == name).unwrap();
                if params.len() != args.len() {
                    return Err(format!("{name} の引数は{}個", params.len()).into());
                }
                for (p, a) in params.iter().zip(args) {
                    self.check(a, &prim(p))?;
                }
                if *ret == "Vec<str>" {
                    V::VecT(Rc::new(prim("str")))
                } else {
                    prim(ret)
                }
            }
            Expr::For(x, it, end, body) => {
                // 繰り返しの中で代入される変数についての事実は、2回目以降には成り立たない
                let mut names = Vec::new();
                assigned(e, &mut names);
                for n in &names {
                    self.forget(n)?;
                }
                self.push();
                let r = (|| {
                    match end {
                        // for i in a..b では a <= i < b
                        Some(end) => {
                            self.check(it, &int)?;
                            self.check(end, &int)?;
                            let (a, b) = (self.eval_value(it)?, self.eval_value(end)?);
                            let id = self.norm.fresh();
                            let v = neu(Neu::Var(id, x.clone()));
                            self.bind(x, int.clone(), v.clone(), false)?;
                            self.add_fact(Norm::bin("<=", a, v.clone()));
                            self.add_fact(Norm::bin("<", v, b));
                        }
                        None => {
                            let V::VecT(el) = base(&self.expr(it)?) else {
                                return Err("for は Vec か a..b に使う".into());
                            };
                            let v = self.norm.opaque();
                            self.bind(x, (*el).clone(), v, false)?;
                        }
                    }
                    self.block(body)
                })();
                self.pop();
                r?;
                for n in &names {
                    self.forget(n)?;
                }
                prim("unit")
            }
            Expr::While(c, body) => {
                want(&boolean, &self.expr(c)?, "while の条件")?;
                // 繰り返しの中で代入される変数についての事実は、2回目以降には成り立たない
                let mut names = Vec::new();
                assigned(e, &mut names);
                for n in &names {
                    self.forget(n)?;
                }
                let cv = self.eval_value(c)?;
                self.enter_branch(&cv, true);
                let r = self.block(body);
                self.pop();
                r?;
                for n in &names {
                    self.forget(n)?;
                }
                // 抜けた後は条件が偽
                self.add_fact(Norm::un("!", cv));
                prim("unit")
            }
            Expr::Block(b) => self.block(b)?,
            Expr::Return(x) => {
                let ret = self.ret.clone();
                match x {
                    Some(x) => self.check(x, &ret)?,
                    None => want(&ret, &prim("unit"), "return")?,
                }
                prim("never")
            }
        })
    }

    // 添字付きの型族（構築子が戻り値の型を指定する型）か。こうした型の値は証明になりうる
    fn is_proof_type(&self, t: &V) -> bool {
        match base(t) {
            V::DataT(d) => self.norm.datas[d.0.as_str()]
                .ctors
                .iter()
                .any(|c| c.ret.is_some()),
            _ => false,
        }
    }

    // v は match で p から取り出した部分（を何段か辿ったもの）か
    fn is_subterm(&self, v: &V, p: &V) -> bool {
        let (V::Neu(v), V::Neu(p)) = (v, p) else {
            return false;
        };
        let (Neu::Var(id, _), Neu::Var(pid, _)) = (&**v, &**p) else {
            return false;
        };
        let mut id = *id;
        while let Some(parent) = self.sub.get(&id) {
            if parent == pid {
                return true;
            }
            id = *parent;
        }
        false
    }

    // 再帰呼び出しで、少なくとも1つの引数がすべての呼び出しを通して小さくなっているか
    fn check_decrease(&mut self, vals: &[V]) -> R<()> {
        for (i, v) in vals.iter().enumerate() {
            if !self.decr[i] {
                continue;
            }
            let (p, t) = self.cur_params[i].clone();
            self.decr[i] = match base(&t) {
                // int は 0 以上で、元の値より小さければよい
                V::Prim("int") => {
                    let goal = Norm::bin(
                        "&&",
                        Norm::bin("<=", V::Int(0), v.clone()),
                        Norm::bin("<", v.clone(), p),
                    );
                    self.prove(&goal)
                }
                V::DataT(_) => self.is_subterm(v, &p),
                _ => false,
            };
        }
        if self.decr.iter().any(|d| *d) {
            Ok(())
        } else {
            Err("再帰呼び出しで小さくなる引数が無いので、停止することを示せない（match で取り出した部分か、0 以上で元より小さい int を渡す。検査しないなら partial fn にする）".into())
        }
    }

    // トップレベルの関数の呼び出し。ジェネリック引数を省いた時は、メタ変数にして引数と期待する型から求める
    fn call_fn(&mut self, name: &str, args: &[Expr], expected: Option<&V>) -> R<V> {
        let f = self.norm.fns[name];
        let explicit = f.params.len() - f.implicit;
        let infer = f.implicit > 0 && args.len() == explicit;
        if !infer && args.len() != f.params.len() {
            return Err(format!("関数 {name} の引数は{explicit}個").into());
        }
        // 引数の型は前の引数の値に依存しうるので、値を順に積みながら計算する
        let mut fenv: Env = vec![HashMap::new()];
        let mut metas = Vec::new();
        let mut sol = HashMap::new();
        let params = if infer {
            for (p, _) in &f.params[..f.implicit] {
                let id = self.norm.fresh();
                metas.push((id, p.clone()));
                fenv[0].insert(p.clone(), neu(Neu::Var(id, format!("?{p}"))));
            }
            &f.params[f.implicit..]
        } else {
            &f.params[..]
        };
        let ids: Vec<usize> = metas.iter().map(|m| m.0).collect();
        // 期待する型が分かれば、先に戻り値の型と合わせる
        if infer && let (Some(exp), Some(rt)) = (expected, &f.ret) {
            let mut env1 = fenv.clone();
            for (p, _) in params {
                let v = self.norm.opaque();
                env1[0].insert(p.clone(), v);
            }
            let rv = self.norm.eval_or_opaque(rt, &mut env1)?;
            self.unify(&rv, &base(exp), &ids, &mut sol)?;
        }
        let mut vals = Vec::new();
        for ((pn, pt), a) in params.iter().zip(args) {
            let r = |e: &str| format!("関数 {name} の引数 {pn}: {e}");
            let t = self.norm.eval_or_opaque(pt, &mut fenv)?;
            let t = self.apply(&sol, &t)?;
            if ids
                .iter()
                .any(|id| !sol.contains_key(id) && mentions(&t, *id))
            {
                let at = self.expr(a).ctx(r)?;
                if !self.unify(&t, &at, &ids, &mut sol)? {
                    return Err(r(&format!("{} が必要だが {}", show(&t), show(&at))).into());
                }
            } else {
                self.check(a, &t).ctx(r)?;
            }
            let v = self.eval_value(a)?;
            vals.push(v.clone());
            fenv[0].insert(pn.clone(), v);
        }
        if infer {
            let mut solved = Vec::new();
            for (id, p) in &metas {
                let Some(v) = sol.get(id) else {
                    return Err(format!(
                        "関数 {name} の型引数 {p} を推論できない。{name}<...>(...) で明示する"
                    )
                    .into());
                };
                let v = self.apply(&sol, v)?;
                fenv[0].insert(p.clone(), v.clone());
                solved.push(v);
            }
            solved.extend(vals);
            vals = solved;
        }
        if self.cur_fn.as_deref() == Some(name) {
            self.check_decrease(&vals)?;
        }
        let ret = match &f.ret {
            Some(t) => self.norm.eval_or_opaque(t, &mut fenv)?,
            None => prim("unit"),
        };
        self.apply(&sol, &ret)
    }

    // トップレベルの関数を値として使った時の型
    fn fn_type(&mut self, f: &Fn) -> R<V> {
        if self.cur_fn.as_deref() == Some(f.name.as_str()) {
            return Err(format!(
                "再帰する関数 {} を値として使えない（停止性を検査できないため）",
                f.name
            )
            .into());
        }
        let ret = f.ret.clone().unwrap_or(Expr::Var("unit".into()));
        let mut env: Env = vec![HashMap::new()];
        match self.norm.eval_pi(&f.params, &ret, &mut env) {
            Ok(v) => Ok(v),
            Err(Halt::Stuck) => Ok(self.norm.opaque()),
            Err(Halt::Err(e)) => Err(e),
        }
    }

    // 関数型 t の値を args で呼んだ時の型。引数の型は前の引数の値に依存しうる
    fn apply_type(&mut self, t: &V, args: &[Expr]) -> R<V> {
        let V::Pi(p) = base(t) else {
            return Err(format!("関数でない値（{}）を呼んでいる", show(t)).into());
        };
        if p.doms.len() != args.len() {
            return Err(format!("引数は{}個", p.doms.len()).into());
        }
        let mut vals: Vec<V> = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let mut d = p.doms[i].clone();
            for (j, v) in vals.iter().enumerate() {
                d = self.norm.subst(&d, binder(p.level, j), v)?;
            }
            self.check(a, &d)
                .ctx(|e| format!("{} 番目の引数: {e}", i + 1))?;
            vals.push(self.eval_value(a)?);
        }
        let mut c = p.cod.clone();
        for (j, v) in vals.iter().enumerate() {
            c = self.norm.subst(&c, binder(p.level, j), v)?;
        }
        Ok(c)
    }

    // 関数の本体を、戻り値の型 rt で検査する
    fn fn_body(&mut self, body: &Block, rt: Option<V>) -> R<V> {
        if let Some(V::Univ(_)) = rt.as_ref().map(base) {
            return Err("関数は型を返せない。型を計算するなら type で定義する".into());
        }
        let saved = self.ret.clone();
        let r = match rt {
            Some(rt) => {
                self.ret = rt.clone();
                if rt == prim("unit") {
                    self.block(body).map(|_| rt)
                } else {
                    self.check_block(body, &rt).map(|_| rt)
                }
            }
            None => {
                // 戻り値の型が無いと return の型が分からない
                self.ret = self.norm.opaque();
                self.block(body)
            }
        };
        self.ret = saved;
        if let Ok(V::Univ(_)) = r.as_ref().map(base) {
            return Err("関数は型を返せない。型を計算するなら type で定義する".into());
        }
        r
    }

    // 無名関数の型を、書かれた引数の型から求める
    fn lam_infer(
        &mut self,
        params: &[(String, Option<Expr>)],
        ret: &Option<Box<Expr>>,
        body: &Block,
    ) -> R<V> {
        let level = self.norm.pi_depth;
        self.norm.pi_depth += 1;
        self.push();
        let r = self.lam_infer_inner(level, params, ret, body);
        self.pop();
        self.norm.pi_depth -= 1;
        r
    }

    fn lam_infer_inner(
        &mut self,
        level: usize,
        params: &[(String, Option<Expr>)],
        ret: &Option<Box<Expr>>,
        body: &Block,
    ) -> R<V> {
        let mut doms = Vec::new();
        let mut ids = Vec::new();
        for (n, t) in params {
            let Some(t) = t else {
                return Err(format!(
                    "無名関数の引数 {n} の型を書く（期待する型が無いので推論できない）"
                )
                .into());
            };
            let ty = self.eval_type(t)?;
            let id = self.norm.fresh();
            self.bind(n, ty.clone(), neu(Neu::Var(id, n.clone())), false)?;
            doms.push(ty);
            ids.push(id);
        }
        let rt = match ret {
            Some(t) => Some(self.eval_type(t)?),
            None => None,
        };
        let cod = self.fn_body(body, rt)?;
        // 引数の変数を、関数型の引数の番号に置き換える
        let mut out = Vec::new();
        for (i, d) in doms.iter().enumerate() {
            let mut d = d.clone();
            for (j, id) in ids[..i].iter().enumerate() {
                d = self.norm.subst(
                    &d,
                    *id,
                    &neu(Neu::Var(binder(level, j), params[j].0.clone())),
                )?;
            }
            out.push(d);
        }
        let mut cod = cod;
        for (j, id) in ids.iter().enumerate() {
            cod = self.norm.subst(
                &cod,
                *id,
                &neu(Neu::Var(binder(level, j), params[j].0.clone())),
            )?;
        }
        Ok(V::Pi(Rc::new(PiV {
            doms: out,
            cod,
            level,
        })))
    }

    // 無名関数を、期待する関数型 p で検査する。引数の型は省いてよい
    fn lam_check(
        &mut self,
        params: &[(String, Option<Expr>)],
        ret: &Option<Box<Expr>>,
        body: &Block,
        p: &PiV,
    ) -> R<()> {
        if params.len() != p.doms.len() {
            return Err(format!("引数は{}個", p.doms.len()).into());
        }
        self.norm.pi_depth += 1;
        self.push();
        let r = (|| {
            let mut vals: Vec<V> = Vec::new();
            for (i, (n, t)) in params.iter().enumerate() {
                let mut d = p.doms[i].clone();
                for (j, v) in vals.iter().enumerate() {
                    d = self.norm.subst(&d, binder(p.level, j), v)?;
                }
                if let Some(t) = t {
                    let tt = self.eval_type(t)?;
                    want(&tt, &d, &format!("引数 {n}"))?;
                }
                let id = self.norm.fresh();
                let v = neu(Neu::Var(id, n.clone()));
                self.bind(n, d, v.clone(), false)?;
                vals.push(v);
            }
            let mut cod = p.cod.clone();
            for (j, v) in vals.iter().enumerate() {
                cod = self.norm.subst(&cod, binder(p.level, j), v)?;
            }
            if let Some(rt) = ret {
                let rt = self.eval_type(rt)?;
                want(&cod, &rt, "戻り値")?;
            }
            self.fn_body(body, Some(cod)).map(|_| ())
        })();
        self.pop();
        self.norm.pi_depth -= 1;
        r
    }

    // 関数型 (x: A, ...) -> B の型。引数と戻り値の型の階層の最大
    fn pi_type(&mut self, binders: &[(String, Expr)], ret: &Expr) -> R<V> {
        self.push();
        let r = (|| {
            let mut level = 0;
            for (n, t) in binders {
                level = level.max(self.type_level(t)?);
                let ty = self.eval_value(t)?;
                let id = self.norm.fresh();
                self.bind(n, ty, neu(Neu::Var(id, n.clone())), false)?;
            }
            Ok(V::Univ(level.max(self.type_level(ret)?)))
        })();
        self.pop();
        r
    }

    // 式が型なら、その型の階層を返す
    fn type_level(&mut self, t: &Expr) -> R<u32> {
        match base(&self.expr(t)?) {
            V::Univ(k) => Ok(k),
            other => Err(format!("型が必要だが {}", show(&other)).into()),
        }
    }

    // data の階層は、フィールドの型の階層の最大。引数の型は数えない
    fn data_level(&mut self, d: &Data) -> R<u32> {
        self.scopes = vec![Scope::default()];
        for (n, t) in &d.params {
            let ty = self
                .eval_type(t)
                .ctx(|e| format!("型 {} の引数 {n}: {e}", d.name))?;
            let id = self.norm.fresh();
            self.bind(n, ty, neu(Neu::Var(id, n.clone())), false)?;
        }
        let mut level = 0;
        for c in &d.ctors {
            let r = |e: &str| format!("構築子 {}: {e}", c.name);
            self.push();
            for (n, t) in &c.fields {
                level = level.max(self.type_level(t).ctx(r)?);
                let ty = self.eval_value(t)?;
                if negative(&ty, &d.name) {
                    return Err(r(&format!(
                        "型 {} が関数型の引数の側に現れる（正値でない型は矛盾を導けるので禁止）",
                        d.name
                    ))
                    .into());
                }
                let id = self.norm.fresh();
                self.bind(n, ty, neu(Neu::Var(id, n.clone())), false)?;
            }
            if let Some(rt) = &c.ret {
                let rt = self.eval_type(rt).ctx(r)?;
                if !matches!(&rt, V::DataT(x) if x.0 == d.name) {
                    return Err(r(&format!("戻り値は {} 型でなければならない", d.name)).into());
                }
            }
            self.pop();
        }
        Ok(level)
    }

    // 型名に引数を渡して型を作る。引数の型は前の引数に依存しうる
    fn data_app(&mut self, name: &str, args: &[Expr]) -> R<V> {
        let d = self.norm.datas[name];
        if d.params.len() != args.len() {
            return Err(format!("型 {name} の引数は{}個", d.params.len()).into());
        }
        let mut env: Env = vec![HashMap::new()];
        for ((pn, pt), a) in d.params.iter().zip(args) {
            let t = self.norm.eval_or_opaque(pt, &mut env)?;
            self.check(a, &t)
                .ctx(|e| format!("型 {name} の引数 {pn}: {e}"))?;
            let v = self.eval_value(a)?;
            env[0].insert(pn.clone(), v);
        }
        Ok(V::Univ(self.levels[name]))
    }

    // 構築子の戻り値の型。env には型の引数とフィールドの値が入っている
    fn ctor_ret(&mut self, d: &Data, c: &Ctor, env: &mut Env) -> R<V> {
        match &c.ret {
            Some(r) => self.norm.eval_or_opaque(r, env),
            None => {
                let args = d.params.iter().map(|(p, _)| env[0][p].clone()).collect();
                Ok(V::DataT(Rc::new((d.name.clone(), args))))
            }
        }
    }

    fn apply(&mut self, sol: &HashMap<usize, V>, v: &V) -> R<V> {
        let mut v = v.clone();
        // 解が別の解を含むことがあるので2回当てる
        for _ in 0..2 {
            for (id, x) in sol {
                v = self.norm.subst(&v, *id, x)?;
            }
        }
        Ok(v)
    }

    // a と b を等しくするメタ変数の解を sol に足す。等しくできなければ false
    fn unify(&mut self, a: &V, b: &V, metas: &[usize], sol: &mut HashMap<usize, V>) -> R<bool> {
        let (a, b) = (self.apply(sol, a)?, self.apply(sol, b)?);
        if a == b {
            return Ok(true);
        }
        for (x, y) in [(&a, &b), (&b, &a)] {
            if let V::Neu(n) = x
                && let Neu::Var(id, _) = &**n
                && metas.contains(id)
            {
                sol.insert(*id, base(y));
                return Ok(true);
            }
        }
        Ok(match (&a, &base(&b)) {
            (V::DataT(x), V::DataT(y)) | (V::Data(x), V::Data(y))
                if x.0 == y.0 && x.1.len() == y.1.len() =>
            {
                for (p, q) in x.1.iter().zip(&y.1) {
                    if !self.unify(p, q, metas, sol)? {
                        return Ok(false);
                    }
                }
                true
            }
            (V::VecT(x), V::VecT(y)) => self.unify(x, y, metas, sol)?,
            (V::Pi(x), V::Pi(y)) if x.doms.len() == y.doms.len() => {
                for (p, q) in x.doms.iter().zip(&y.doms).chain([(&x.cod, &y.cod)]) {
                    if !self.unify(p, q, metas, sol)? {
                        return Ok(false);
                    }
                }
                true
            }
            _ => same(&a, &b),
        })
    }

    // 構築子の呼び出し。型の引数はメタ変数にして、期待する型と引数の型から求める
    fn ctor_call(&mut self, name: &str, args: &[Expr], expected: Option<&V>) -> R<V> {
        let (d, c) = self.norm.ctors[name];
        if c.fields.len() != args.len() {
            return Err(format!("構築子 {name} の引数は{}個", c.fields.len()).into());
        }
        let mut env: Env = vec![HashMap::new()];
        let mut metas = Vec::new();
        for (p, _) in &d.params {
            let id = self.norm.fresh();
            metas.push((id, p.clone()));
            env[0].insert(p.clone(), neu(Neu::Var(id, format!("?{p}"))));
        }
        let mut field_metas = HashMap::new();
        let mut sol = HashMap::new();
        if let Some(exp) = expected {
            // フィールドもメタ変数にして戻り値の型を作り、期待する型と合わせる
            let mut env1 = env.clone();
            for (f, _) in &c.fields {
                let id = self.norm.fresh();
                field_metas.insert(f.clone(), id);
                env1[0].insert(f.clone(), neu(Neu::Var(id, format!("?{f}"))));
            }
            let ret1 = self.ctor_ret(d, c, &mut env1)?;
            let ids: Vec<usize> = metas
                .iter()
                .map(|m| m.0)
                .chain(field_metas.values().copied())
                .collect();
            if !self.unify(&ret1, &base(exp), &ids, &mut sol)? {
                return Err(format!(
                    "{} が必要だが、{name} の型は {}",
                    show(exp),
                    show(&self.apply(&sol, &ret1)?)
                )
                .into());
            }
        }
        let ids: Vec<usize> = metas.iter().map(|m| m.0).collect();
        for ((f, ft), a) in c.fields.iter().zip(args) {
            let r = |e: &str| format!("構築子 {name} の引数 {f}: {e}");
            let t = self.norm.eval_or_opaque(ft, &mut env)?;
            let t = self.apply(&sol, &t)?;
            if ids
                .iter()
                .any(|id| !sol.contains_key(id) && mentions(&t, *id))
            {
                let at = self.expr(a).ctx(r)?;
                if !self.unify(&t, &at, &ids, &mut sol)? {
                    return Err(r(&format!("{} が必要だが {}", show(&t), show(&at))).into());
                }
            } else {
                self.check(a, &t).ctx(r)?;
            }
            let v = self.eval_value(a)?;
            if let Some(id) = field_metas.get(f)
                && let Some(want_v) = sol.get(id).cloned()
                && !self.unify(&want_v, &v, &ids, &mut sol)?
            {
                return Err(r(&format!("{} が必要だが {}", show(&want_v), show(&v))).into());
            }
            env[0].insert(f.clone(), v);
        }
        let ret = self.ctor_ret(d, c, &mut env)?;
        let ret = self.apply(&sol, &ret)?;
        if let Some((_, p)) = metas.iter().find(|(id, _)| mentions(&ret, *id)) {
            return Err(format!("構築子 {name} の型引数 {p} を推論できない。型注釈を書く").into());
        }
        Ok(ret)
    }

    // 添字を照合する。match する値の側の変数は、構築子の添字に置き換える代入として out に積む。
    // 照合できない（別の構築子・別の定数）なら false で、その場合は起こらない
    fn unify_index(&mut self, c: &V, s: &V, out: &mut Vec<(usize, V)>) -> R<bool> {
        let (mut c, mut s) = (c.clone(), s.clone());
        for (id, v) in out.iter() {
            c = self.norm.subst(&c, *id, v)?;
            s = self.norm.subst(&s, *id, v)?;
        }
        if c == s {
            return Ok(true);
        }
        for (x, y) in [(&s, &c), (&c, &s)] {
            if let V::Neu(n) = x
                && let Neu::Var(id, _) = &**n
                && *id != 0
            {
                out.push((*id, y.clone()));
                return Ok(true);
            }
        }
        Ok(match (&c, &s) {
            (V::Data(x), V::Data(y)) | (V::DataT(x), V::DataT(y)) => {
                if x.0 != y.0 || x.1.len() != y.1.len() {
                    return Ok(false);
                }
                for (p, q) in x.1.iter().zip(&y.1) {
                    if !self.unify_index(p, q, out)? {
                        return Ok(false);
                    }
                }
                true
            }
            (V::Neu(_), _) | (_, V::Neu(_)) => true,
            _ => false,
        })
    }

    // 構築子 c の場合に入る。フィールドを束縛し、添字の照合で得た代入を返す。起こらない場合は None
    fn enter_ctor(
        &mut self,
        d: &Data,
        c: &Ctor,
        sargs: &[V],
        binds: Option<&[String]>,
    ) -> R<Option<Entered>> {
        let mut env: Env = vec![
            d.params
                .iter()
                .map(|(p, _)| p.clone())
                .zip(sargs.iter().cloned())
                .collect(),
        ];
        let mut fvals = Vec::new();
        for (i, (f, ft)) in c.fields.iter().enumerate() {
            let t = self.norm.eval_or_opaque(ft, &mut env)?;
            let b = binds.map_or("_", |b| b[i].as_str());
            let id = self.norm.fresh();
            let v = neu(Neu::Var(
                id,
                if b == "_" { f.clone() } else { b.to_string() },
            ));
            env[0].insert(f.clone(), v.clone());
            fvals.push(v.clone());
            if b != "_" {
                self.bind(b, t, v, false)?;
            }
        }
        let V::DataT(rt) = self.ctor_ret(d, c, &mut env)? else {
            return Err(format!("構築子 {} の戻り値の型が計算できない", c.name).into());
        };
        let mut out = Vec::new();
        for (x, y) in rt.1.iter().zip(sargs) {
            if !self.unify_index(x, y, &mut out)? {
                return Ok(None);
            }
        }
        Ok(Some((out, fvals)))
    }

    // 変数 id の値が v だと分かった。id を含む変数の型と値、事実を置き換える
    fn refine_var(&mut self, id: usize, v: &V) -> R<()> {
        let mut seen = Vec::new();
        let mut redo = Vec::new();
        for s in self.scopes.iter().rev() {
            for (n, e) in &s.vars {
                if seen.contains(n) {
                    continue;
                }
                seen.push(n.clone());
                if mentions(&e.ty, id) || mentions(&e.val, id) {
                    redo.push((n.clone(), e.ty.clone(), e.val.clone(), e.mutable));
                }
            }
        }
        let facts: Vec<V> = self
            .scopes
            .iter()
            .flat_map(|s| &s.facts)
            .filter(|f| mentions(f, id))
            .cloned()
            .collect();
        for (n, ty, val, mutable) in redo {
            let (ty, val) = (self.norm.subst(&ty, id, v)?, self.norm.subst(&val, id, v)?);
            self.scopes
                .last_mut()
                .unwrap()
                .vars
                .insert(n, Entry { ty, val, mutable });
        }
        for f in facts {
            let f = self.norm.subst(&f, id, v)?;
            self.add_fact(f);
        }
        Ok(())
    }

    // match を検査する。各場合では、添字の照合と、値が構築子の形であることで期待する型を絞り込む
    fn check_match(&mut self, s: &Expr, arms: &[Arm], expected: Option<&V>) -> R<V> {
        let st = base(&self.expr(s)?);
        let V::DataT(dt) = &st else {
            return Err(format!("match は data 型の値に使う（実際は {}）", show(&st)).into());
        };
        let d = self.norm.datas[dt.0.as_str()];
        let sv = self.eval_value(s)?;
        let svar = match &sv {
            V::Neu(n) => match &**n {
                Neu::Var(id, _) => Some(*id),
                _ => None,
            },
            _ => None,
        };
        let mut covered = Vec::new();
        let mut wildcard = false;
        let mut result: Option<V> = None;
        for arm in arms {
            let r = |e: &str| format!("{} の場合: {e}", arm.ctor);
            self.push();
            let mut exp = expected.cloned();
            if arm.ctor == "_" {
                wildcard = true;
            } else {
                let Some((dd, c)) = self.norm.ctors.get(arm.ctor.as_str()).copied() else {
                    self.pop();
                    return Err(format!("不明な構築子 {}", arm.ctor).into());
                };
                if dd.name != d.name || arm.binds.len() != c.fields.len() {
                    self.pop();
                    return Err(format!(
                        "{} は {} の構築子でないか、フィールドの数が違う",
                        arm.ctor, d.name
                    )
                    .into());
                }
                covered.push(c.name.clone());
                let Some((mut substs, fvals)) =
                    self.enter_ctor(d, c, &dt.1, Some(&arm.binds)).ctx(r)?
                else {
                    // 添字が合わないので、この場合は起こらない
                    self.pop();
                    continue;
                };
                // 取り出したフィールドは match した値より小さい（停止性の検査に使う）
                if let Some(sid) = svar {
                    for fv in &fvals {
                        if let V::Neu(n) = fv
                            && let Neu::Var(fid, _) = &**n
                        {
                            self.sub.insert(*fid, sid);
                        }
                    }
                }
                if let Some(id) = svar {
                    substs.push((id, V::Data(Rc::new((c.name.clone(), fvals)))));
                }
                for (id, v) in &substs {
                    if let Some(e) = &mut exp {
                        *e = self.norm.subst(e, *id, v)?;
                    }
                    self.refine_var(*id, v)?;
                }
            }
            let t = match &exp {
                Some(e) => self.check(&arm.body, e).map(|_| e.clone()),
                None => self.expr(&arm.body),
            };
            self.pop();
            let t = t.ctx(r)?;
            if expected.is_some() {
                continue;
            }
            result = Some(match result {
                None => t,
                Some(prev) if prev == prim("never") => t,
                Some(prev) => {
                    want(&base(&prev), &t, &r("場合ごとの型"))?;
                    base(&prev)
                }
            });
        }
        if !wildcard {
            for c in &d.ctors {
                if covered.contains(&c.name) {
                    continue;
                }
                self.push();
                let possible = self.enter_ctor(d, c, &dt.1, None)?.is_some();
                self.pop();
                if possible {
                    return Err(format!("match に {} の場合が無い", c.name).into());
                }
            }
        }
        Ok(expected.cloned().or(result).unwrap_or(prim("never")))
    }

    // 構造体の値 x のフィールド f の型。フィールドの型は前のフィールドの値に依存しうる
    fn struct_field(&mut self, d: &(String, Vec<V>), x: &Expr, f: &str) -> R<V> {
        let data = self.norm.datas[d.0.as_str()];
        let [c] = data.ctors.as_slice() else {
            return Err(format!("型 {} は構造体でないのでフィールドは無い", d.0).into());
        };
        if !c.name.ends_with("{}") {
            return Err(format!("型 {} は構造体でないのでフィールドは無い", d.0).into());
        }
        let xv = self.eval_value(x)?;
        let mut env: Env = vec![
            data.params
                .iter()
                .map(|p| p.0.clone())
                .zip(d.1.iter().cloned())
                .collect(),
        ];
        for (i, (n, ft)) in c.fields.iter().enumerate() {
            let t = self.norm.eval_or_opaque(ft, &mut env)?;
            if n == f {
                return Ok(t);
            }
            let v = match &xv {
                V::Data(dv) => dv.1[i].clone(),
                _ => self.norm.opaque(),
            };
            env[0].insert(n.clone(), v);
        }
        Err(format!("構造体 {} にフィールド {f} は無い", d.0).into())
    }

    // syn<種類> のフィールド f の型。種類が syntax 規則なら、そのフィールドがあることを確かめる
    fn syn_field(&self, t: &V, f: &str) -> R<V> {
        let V::Syn(k) = t else { return Ok(prim("syn")) };
        if k.starts_with('[') {
            return Err(format!("syn<{k}> は配列なので、要素は at で取り出す").into());
        }
        match self.rules.get(&**k) {
            Some(fs) => match fs.iter().find(|x| x.0 == f) {
                Some((_, fk)) => Ok(syn_of(fk)),
                None => Err(format!(
                    "syntax {k} にフィールド {f} は無い（あるのは {}）",
                    fs.iter()
                        .map(|x| x.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .into()),
            },
            None => Err(format!("syn<{k}> にフィールドは無い").into()),
        }
    }

    fn check_q(&mut self, q: &[Q]) -> R<()> {
        for x in q {
            match x {
                Q::Tok(_) => {}
                Q::Var { path, .. } => {
                    let t = self
                        .lookup(&path[0])
                        .ok_or(format!("未定義の変数 {}", path[0]))?
                        .ty
                        .clone();
                    if path.len() > 1 {
                        want(&prim("syn"), &t, &format!("${}", path.join(".")))?;
                        // $( )* の中では配列の要素を指すので、配列の括弧を外して辿る
                        let mut t = base(&t);
                        for f in &path[1..] {
                            if let V::Syn(k) = &t {
                                let k = k.trim_start_matches('[').trim_end_matches(']').to_string();
                                t = self.syn_field(&V::Syn(k.into()), f)?;
                            }
                        }
                    }
                }
                Q::Splice(_, e) => {
                    let t = self.expr(e)?;
                    if t == prim("unit") || t == prim("never") {
                        return Err("${ } の値を差し込めない".into());
                    }
                }
                Q::Rep(inner, _) => {
                    // 繰り返しの中では、配列の束縛はその要素を指す
                    let mut names = Vec::new();
                    q_names(inner, &mut names);
                    self.push();
                    for n in names {
                        if let Some(V::Syn(k)) = self.lookup(&n).map(|e| base(&e.ty))
                            && k.starts_with('[')
                        {
                            let v = self.norm.opaque();
                            self.bind(&n, syn_of(&k[1..k.len() - 1]), v, false)?;
                        }
                    }
                    let r = self.check_q(inner);
                    self.pop();
                    r?
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse;

    fn check(src: &str) -> crate::error::R<()> {
        super::check(&parse(src)?)
    }

    #[test]
    fn dependent() {
        // 引数の値で戻り値の型が決まる
        let src = r#"
type T<b: bool> = if b { int } else { str }
fn f(b: bool) -> T<b> { if b { 1 } else { "one" } }
fn id(A: Type, x: A) -> A { x }
fn main() {
    let x: int = f(true)
    let s: str = f(false)
    let y: T<(1 < 2)> = id(int, 3)
    print(x + y)
}
"#;
        check(src).unwrap();
        for (bad, msg) in [
            ("let x: int = f(true)", "let x: int = f(false)"),
            (
                "let y: T<(1 < 2)> = id(int, 3)",
                "let y: T<(1 < 2)> = id(int, true)",
            ),
            ("let s: str = f(false)", "let s: str = f(1 == 1)"),
        ] {
            let e = check(&src.replace(bad, msg)).unwrap_err();
            assert!(e.contains("が必要"), "{msg} => {e}");
        }
        let e = check(&src.replace("{ \"one\" }", "{ 2 }")).unwrap_err();
        assert!(e.contains("str が必要だが int"), "{e}");
    }

    #[test]
    fn type_level_loop_is_bounded() {
        let e =
            check("type Loop<n: int> = Loop<n + 1>\nfn main() { let x: Loop<0> = 1 }").unwrap_err();
        assert!(e.contains("停止することを示せない"), "{e}");
        // 関数は型を返せない
        let e = check("fn f() -> Type { int } fn main() {}").unwrap_err();
        assert!(e.contains("関数は型を返せない"), "{e}");
        let e = check("fn main() { let f = fn() { int } }").unwrap_err();
        assert!(e.contains("関数は型を返せない"), "{e}");
        // 型には <> で引数を渡す
        let e =
            check("type Box<A: Type> = | MkBox(x: A)\nfn main() { let b: Box(int) = MkBox(1) }")
                .unwrap_err();
        assert!(e.contains("<> で引数を渡す"), "{e}");
    }

    #[test]
    fn inductive() {
        let src = include_str!("../examples/inductive.hb");
        check(src).unwrap();
        let extra = "type TBox = | MkT(t: Type)\nfn id(A: Type, x: A) -> A { x }\nfn two() -> Eq<int, 1 + 1, 2> { Refl }\n";
        check(&format!("{src}{extra}")).unwrap();
        for (from, to, msg) in [
            ("Succ(k) => Succ(add(k, n))", "Succ(k) => n", "が必要だが"),
            ("        Some(x) => x\n", "", "Some の場合が無い"),
            (
                "Succ(k) => cong(add(k, Zero), k, add_zero(k))",
                "Succ(k) => Refl",
                "が必要だが",
            ),
            ("let o = Some(3)", "let o = None", "推論できない"),
            (
                "fn zero_add(n: Nat) -> Eq<Nat, add(Zero, n), n>",
                "fn zero_add(n: Nat) -> Eq<Nat, add(n, Zero), n>",
                "が必要だが",
            ),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
        // Type を含む data は Type<1> なので、Type を求める所には渡せない
        let e = check(&format!(
            "{src}{extra}fn f() -> int {{ let b = id(TBox, MkT(int))\n 0 }}"
        ))
        .unwrap_err();
        assert!(e.contains("Type が必要だが Type<1>"), "{e}");
        let e = check(&format!("{src}{extra}fn bad() -> Eq<int, 1, 2> {{ Refl }}")).unwrap_err();
        assert!(e.contains("が必要だが"), "{e}");
        // 停止性: 止まらない関数で偽の証明を作れない
        for (bad, msg) in [
            (
                "fn bad() -> Eq<int, 1, 2> { bad() }",
                "停止することを示せない",
            ),
            ("partial fn bad() -> Eq<int, 1, 2> { bad() }", "証明の型"),
            ("fn bad(n: Nat) -> Nat { bad(n) }", "停止することを示せない"),
            (
                "fn bad(n: int) -> int { bad(n - 1) }",
                "停止することを示せない",
            ),
            (
                "fn ev(n: Nat) -> bool { match n { Zero => true, Succ(k) => od(k) } }\nfn od(n: Nat) -> bool { match n { Zero => false, Succ(k) => ev(k) } }",
                "互いに呼び合っている",
            ),
        ] {
            let e = check(&format!("{src}{bad}")).unwrap_err();
            assert!(e.contains(msg), "{bad} => {e}");
        }
        check(&format!(
            "{src}partial fn spin(n: int) -> int {{ spin(n) }}"
        ))
        .unwrap();
        check(&format!(
            "{src}fn down(n: int) -> int {{ if n > 0 {{ down(n - 1) }} else {{ 0 }} }}"
        ))
        .unwrap();
    }

    #[test]
    fn closure() {
        let src = include_str!("../examples/closure.hb");
        check(src).unwrap();
        for (from, to, msg) in [
            ("print(twice(inc)(3))", "print(twice(add)(3))", "が必要だが"),
            (
                "fn(b) { if b { 1 } else { \"no\" } }",
                "fn(b) { if b { \"no\" } else { 1 } }",
                "int が必要だが str",
            ),
            (
                "let inc = fn(x: int) -> int { x + 1 }",
                "let inc = fn(x) { x + 1 }",
                "型を書く",
            ),
            ("print(f(2, 3))", "print(f(2))", "引数は2個"),
            ("print(adder(10)(5))", "print(inc(10)(5))", "関数でない値"),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
        // ジェネリック引数: 推論できなければ明示を求める。期待する型からも推論する
        let g = "type Opt<A> = Non | Som(x: A)\nfn non<A>() -> Opt<A> { Non }\nfn id<A>(x: A) -> A { x }\n";
        check(&format!(
            "{g}fn main() {{ let x: Opt<int> = non()\n let y = non<int>()\n let z: int = id(3) }}"
        ))
        .unwrap();
        for (body, msg) in [
            ("let x = non()", "推論できない"),
            ("let y: int = id<int>(\"s\")", "int が必要だが str"),
            ("let z: str = id(3)", "str が必要だが int"),
            ("let w = id<int, int>(3)", "引数は1個"),
        ] {
            let e = check(&format!("{g}fn main() {{ {body} }}")).unwrap_err();
            assert!(e.contains(msg), "{body} => {e}");
        }
        // 正値性: 自分を関数型の引数に持つ型は矛盾を導ける
        let e = check("type Bad = | MkBad(f: (Bad) -> int)\nfn main() {}").unwrap_err();
        assert!(e.contains("正値でない"), "{e}");
        check("type Ok = | MkOk(f: (int) -> Ok)\nfn main() {}").unwrap();
        // 再帰する関数を値にすると、減る引数の検査をすり抜けられる
        let e = check(&format!(
            "{src}fn bad(n: int) -> int {{ let g = bad\n g(n) }}"
        ))
        .unwrap_err();
        assert!(e.contains("値として使えない"), "{e}");
    }

    #[test]
    fn structs() {
        let src = include_str!("../examples/struct.hb");
        check(src).unwrap();
        for (from, to, msg) in [
            (
                "Point { x: 1, y: 2 }",
                "Point { x: 1 }",
                "フィールド y が無い",
            ),
            (
                "Point { x: 1, y: 2 }",
                "Point { x: 1, y: 2, z: 3 }",
                "フィールド z は無い",
            ),
            (
                "Point { x: 1, y: 2 }",
                "Point { x: 1, y: true }",
                "int が必要だが bool",
            ),
            ("Size { w: 3, h: 4 }", "Size { w: 0, h: 4 }", "証明できない"),
            ("print(q.x + q.y)", "print(q.z)", "フィールド z は無い"),
            ("print(pr.snd)", "print(pr.snd + 1)", "str が必要だが int"),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
    }

    #[test]
    fn strings_and_for() {
        let src = include_str!("../examples/strings.hb");
        check(src).unwrap();
        for (from, to, msg) in [
            (
                "for i in 0..len(parts) {",
                "for i in 0..len(parts) + 1 {",
                "添字の範囲",
            ),
            (
                "len(parts) > 2 && parts[2]",
                "len(parts) > 2 || parts[2]",
                "添字の範囲",
            ),
            (
                "\"Hello, \" + name",
                "\"Hello, \" + 1",
                "str が必要だが int",
            ),
            ("for p in parts {", "for p in 3 {", "Vec か a..b"),
            (
                "slice(\"hibana\", 0, 3)",
                "slice(\"hibana\", \"0\", 3)",
                "int が必要だが str",
            ),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
    }

    #[test]
    fn refinement() {
        let src = r#"
type Pos = int[x -> x > 0]
type NumVec<T: Type> = Vec<T>
fn get(v: Vec<int>, i: int[i -> i >= 0]) -> int {
    if i < len(v) { v[i] } else { 0 }
}
fn sum(v: NumVec<Pos>) -> int {
    let mut s = 0
    let mut i: int[x -> x >= 0] = 0
    while i < len(v) {
        s = s + v[i]
        i = i + 1
    }
    s
}
fn add(x: Pos, y: Pos) -> int[r -> r > 1] { x + y }
fn main() {
    let p: Pos = 3
    let v: NumVec<int[x -> x > 0]> = [1, 2, p]
    print(sum(v) + get(v, 1) + v[2])
}
"#;
        check(src).unwrap();
        for (from, to, msg) in [
            ("let p: Pos = 3", "let p: Pos = 0", "証明できない"),
            (
                "-> int[r -> r > 1] { x + y }",
                "-> int[r -> r > 2] { x + y }",
                "証明できない",
            ),
            ("[1, 2, p]", "[1, -2, p]", "証明できない"),
            ("+ v[2]", "+ v[3]", "添字の範囲"),
            ("get(v, 1)", "get(v, 0 - 1)", "証明できない"),
            (
                "if i < len(v) { v[i] }",
                "if i <= len(v) { v[i] }",
                "添字の範囲",
            ),
            (
                "i = i + 1\n",
                "i = i + 1\n        s = s + v[i]\n",
                "添字の範囲",
            ),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
    }
}
