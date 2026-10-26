// 依存型の型検査。型は式であり、検査中に評価（正規化）して比べる
use crate::meta::Q;
use crate::parser::{Block, Expr, Fn, Stmt};
use std::collections::HashMap;
use std::rc::Rc;

type R<T> = Result<T, String>;

// 型検査中の値。型そのものも値として表す
#[derive(Debug, Clone, PartialEq)]
enum V {
    Int(i64),
    Bool(bool),
    Str(Rc<str>),
    Unit,
    // 型の型 Type
    // ponytail: Type : Type なので論理として矛盾する（Girard のパラドックス）。証明に使う時は階層を入れる
    Univ,
    // int・bool・str・syn・unit・never。never は return の型で、どの型とも一致する
    Prim(&'static str),
    // 値が分からず計算が止まった項
    Neu(Rc<Neu>),
}

#[derive(Debug, PartialEq)]
enum Neu {
    // 引数など、値の分からない変数。番号で区別する
    Var(usize, String),
    Un(&'static str, V),
    Bin(&'static str, V, V),
    // 展開しきれなかった関数呼び出し
    Call(String, Vec<V>),
    // 型の計算で扱えない式（while・代入・print など）。どの値とも等しくない
    Opaque(usize),
}

const PRIMS: &[&str] = &["int", "bool", "str", "syn", "unit"];

// マクロ本体の中でだけ使える組み込み関数 (名前, 引数, 戻り値)
const SYN_FNS: &[(&str, &[&str], &str)] = &[
    ("len", &["syn"], "int"),
    ("at", &["syn", "int"], "syn"),
    ("is_null", &["syn"], "bool"),
    ("int_of", &["syn"], "int"),
    ("str_of", &["syn"], "str"),
    ("error", &["str"], "never"),
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

fn show(v: &V) -> String {
    match v {
        V::Int(n) => n.to_string(),
        V::Bool(b) => b.to_string(),
        V::Str(s) => format!("{s:?}"),
        V::Unit => "()".into(),
        V::Univ => "Type".into(),
        V::Prim(p) => p.to_string(),
        V::Neu(n) => match &**n {
            Neu::Var(_, name) => name.clone(),
            Neu::Un(op, x) => format!("{op}{}", show(x)),
            Neu::Bin(op, a, b) => format!("({} {op} {})", show(a), show(b)),
            Neu::Call(f, args) => format!(
                "{f}({})",
                args.iter().map(show).collect::<Vec<_>>().join(", ")
            ),
            Neu::Opaque(_) => "?".into(),
        },
    }
}

fn neu(n: Neu) -> V {
    V::Neu(Rc::new(n))
}

// 評価が止まる理由。Stuck は値の分からない条件で分岐した時など
enum Halt {
    Stuck,
    Err(String),
}

type H<T> = Result<T, Halt>;

// 型の計算をする評価器
// ponytail: 停止性を検査しない。燃料が尽きたらエラーにする
struct Norm<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    fuel: usize,
    // 展開の入れ子の深さ。スタックあふれを防ぐ
    depth: usize,
    next_id: usize,
}

type Env = Vec<HashMap<String, V>>;

impl Norm<'_> {
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
        r.unwrap_or_else(|| neu(self::Neu::Bin(op, a, b)))
    }

    // 関数を展開する。中で止まったら呼び出しのまま残す
    fn call(&mut self, name: &str, args: Vec<V>) -> R<V> {
        let Some(f) = self.fns.get(name).copied() else {
            return Ok(neu(Neu::Call(name.into(), args)));
        };
        if self.fuel == 0 || self.depth >= 200 {
            return Err(format!("型の計算が終わらない（{name} の展開）"));
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
            Expr::Int(n) => V::Int(*n),
            Expr::Bool(b) => V::Bool(*b),
            Expr::Str(s) => V::Str(s.as_str().into()),
            Expr::Var(n) => match env.iter().rev().find_map(|s| s.get(n)) {
                Some(v) => v.clone(),
                None if n == "Type" => V::Univ,
                None if PRIMS.contains(&n.as_str()) => prim(n),
                None => return Err(Halt::Err(format!("未定義の変数 {n}"))),
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
                self.call(name, vs).map_err(Halt::Err)?
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
        let V::Neu(n) = v else { return Ok(v.clone()) };
        Ok(match &**n {
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
        })
    }
}

// 変数の (型, 値, 可変か)
struct Entry {
    ty: V,
    val: V,
    mutable: bool,
}

// ponytail: エラーに行番号が無い。AST にスパンを持たせる時に追加する
struct Ck<'a> {
    norm: Norm<'a>,
    scopes: Vec<HashMap<String, Entry>>,
    ret: V,
    // マクロ本体（コンパイル時に実行するコード）の中か
    in_macro: bool,
}

fn same(a: &V, b: &V) -> bool {
    let never = V::Prim("never");
    a == b || *a == never || *b == never
}

fn want(e: &V, got: &V, what: &str) -> R<()> {
    if same(e, got) {
        Ok(())
    } else {
        Err(format!("{what}: {} が必要だが {}", show(e), show(got)))
    }
}

pub fn check(fns: &[Fn]) -> R<()> {
    let mut ck = Ck {
        norm: Norm {
            fns: HashMap::new(),
            fuel: 100_000,
            depth: 0,
            next_id: 0,
        },
        scopes: Vec::new(),
        ret: V::Unit,
        in_macro: false,
    };
    for f in fns {
        let reserved = f.name == "print" || f.name == "Type" || PRIMS.contains(&f.name.as_str());
        if reserved
            || SYN_FNS.iter().any(|s| s.0 == f.name)
            || ck.norm.fns.insert(&f.name, f).is_some()
        {
            return Err(format!("関数 {} が重複している", f.name));
        }
    }
    match ck.norm.fns.get("main") {
        Some(f) if f.params.is_empty() => {}
        _ => return Err("引数なしの main 関数が必要".into()),
    }
    for f in fns {
        ck.scopes = vec![HashMap::new()];
        for (n, t) in &f.params {
            let ty = ck
                .eval_type(t)
                .map_err(|e| format!("関数 {} の引数 {n}: {e}", f.name))?;
            let id = ck.norm.fresh();
            ck.bind(n, ty, neu(Neu::Var(id, n.clone())), false);
        }
        ck.ret = match &f.ret {
            Some(t) => ck
                .eval_type(t)
                .map_err(|e| format!("関数 {} の戻り値: {e}", f.name))?,
            None => prim("unit"),
        };
        let ret = ck.ret.clone();
        // 戻り値が unit の関数は最後の式の値を捨てる
        if ret == prim("unit") {
            ck.block(&f.body)?;
        } else {
            ck.check_block(&f.body, &ret)
                .map_err(|e| format!("関数 {} の戻り値: {e}", f.name))?;
        }
    }
    Ok(())
}

// マクロ本体を、束縛名を syn 型の変数として検査する。値は syn 型でなければならない
pub fn check_macro(b: &Block, params: &[String]) -> R<()> {
    let mut ck = Ck {
        norm: Norm {
            fns: HashMap::new(),
            fuel: 100_000,
            depth: 0,
            next_id: 0,
        },
        scopes: vec![HashMap::new()],
        ret: prim("syn"),
        in_macro: true,
    };
    for p in params {
        let v = ck.norm.opaque();
        ck.bind(p, prim("syn"), v, false);
    }
    let t = ck.block(b)?;
    want(&prim("syn"), &t, "マクロ本体の値")
}

impl Ck<'_> {
    fn bind(&mut self, name: &str, ty: V, val: V, mutable: bool) {
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.into(), Entry { ty, val, mutable });
    }

    fn lookup(&self, name: &str) -> Option<&Entry> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    // 評価器に渡す、変数の値の環境
    // ponytail: 評価のたびに全スコープを複製する。遅ければ永続的なマップにする
    fn val_env(&self) -> Env {
        vec![
            self.scopes
                .iter()
                .flat_map(|s| s.iter().map(|(k, e)| (k.clone(), e.val.clone())))
                .collect(),
        ]
    }

    fn eval_value(&mut self, e: &Expr) -> R<V> {
        let mut env = self.val_env();
        self.norm.eval_or_opaque(e, &mut env)
    }

    // 式が型であることを確かめてから、その型を計算する
    fn eval_type(&mut self, e: &Expr) -> R<V> {
        want(&V::Univ, &self.expr(e)?, "型")?;
        self.eval_value(e)
    }

    fn block(&mut self, b: &Block) -> R<V> {
        self.scopes.push(HashMap::new());
        let r = self.block_inner(b, None);
        self.scopes.pop();
        r
    }

    fn check_block(&mut self, b: &Block, expected: &V) -> R<()> {
        self.scopes.push(HashMap::new());
        let r = self.block_inner(b, Some(expected));
        self.scopes.pop();
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
                            self.check(init, &t)
                                .map_err(|e| format!("変数 {name}: {e}"))?;
                            t
                        }
                        None => self.expr(init)?,
                    };
                    // 可変な変数の値は型の計算に使わない
                    let val = if *mutable {
                        self.norm.opaque()
                    } else {
                        self.eval_value(init)?
                    };
                    self.bind(name, t, val, *mutable);
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

    // 期待する型に対して検査する。if は条件の値で期待する型を絞り込む
    fn check(&mut self, e: &Expr, expected: &V) -> R<()> {
        match e {
            Expr::If(c, then, Some(els)) => {
                want(&prim("bool"), &self.expr(c)?, "if の条件")?;
                // 条件が値の分からない変数なら、各分岐ではその値が分かっているものとして扱う
                let var = match &**c {
                    Expr::Var(n) => self.lookup(n).and_then(|en| match &en.val {
                        V::Neu(nu) => match &**nu {
                            Neu::Var(id, _) => Some((n.clone(), *id, en.ty.clone())),
                            _ => None,
                        },
                        _ => None,
                    }),
                    _ => None,
                };
                for (b, branch) in [(true, None), (false, Some(&**els))] {
                    let mut exp = expected.clone();
                    self.scopes.push(HashMap::new());
                    if let Some((n, id, ty)) = &var {
                        exp = self.norm.subst(expected, *id, &V::Bool(b))?;
                        self.bind(n, ty.clone(), V::Bool(b), false);
                    }
                    let r = match branch {
                        None => self.check_block(then, &exp),
                        Some(x) => self.check(x, &exp),
                    };
                    self.scopes.pop();
                    r?;
                }
                Ok(())
            }
            Expr::Block(b) => self.check_block(b, expected),
            e => {
                let t = self.expr(e)?;
                want(expected, &t, "式")
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> R<V> {
        let int = prim("int");
        let boolean = prim("bool");
        Ok(match e {
            Expr::Int(_) => int,
            Expr::Bool(_) => boolean,
            Expr::Str(_) => prim("str"),
            Expr::Var(n) => match self.lookup(n) {
                Some(en) => en.ty.clone(),
                None if n == "Type" || PRIMS.contains(&n.as_str()) => V::Univ,
                None => return Err(format!("未定義の変数 {n}")),
            },
            Expr::Field(x, _) => {
                want(&prim("syn"), &self.expr(x)?, "フィールド参照")?;
                prim("syn")
            }
            Expr::Method(..) => return Err("メソッドは未対応".into()),
            Expr::Macro(n, _) => return Err(format!("マクロ {n} に展開コードが無い")),
            Expr::Quote(q) => {
                if !self.in_macro {
                    return Err("quote はマクロ本体の中でだけ使える".into());
                }
                self.check_q(q)?;
                prim("syn")
            }
            Expr::Unary(op, x) => {
                let t = if *op == "-" { int } else { boolean };
                want(&t, &self.expr(x)?, op)?;
                t
            }
            Expr::Binary(op, l, r) => {
                let (lt, rt) = (self.expr(l)?, self.expr(r)?);
                match *op {
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
                        want(&lt, &rt, op)?;
                        if lt == prim("syn") {
                            return Err("syn は比較できない".into());
                        }
                        boolean
                    }
                }
            }
            Expr::Call(name, args) if name == "print" => {
                if args.len() != 1 {
                    return Err("print の引数は1個".into());
                }
                self.expr(&args[0])?;
                prim("unit")
            }
            Expr::Call(name, args) if self.in_macro && SYN_FNS.iter().any(|s| s.0 == name) => {
                let (_, params, ret) = SYN_FNS.iter().find(|s| s.0 == name).unwrap();
                if params.len() != args.len() {
                    return Err(format!("{name} の引数は{}個", params.len()));
                }
                for (p, a) in params.iter().zip(args) {
                    self.check(a, &prim(p))?;
                }
                prim(ret)
            }
            Expr::Call(name, args) => {
                let f = *self
                    .norm
                    .fns
                    .get(name.as_str())
                    .ok_or(format!("未定義の関数 {name}"))?;
                if f.params.len() != args.len() {
                    return Err(format!("関数 {name} の引数は{}個", f.params.len()));
                }
                // 引数の型は前の引数の値に依存しうるので、値を順に積みながら計算する
                let mut fenv: Env = vec![HashMap::new()];
                for ((pn, pt), a) in f.params.iter().zip(args) {
                    let t = self.norm.eval_or_opaque(pt, &mut fenv)?;
                    self.check(a, &t)
                        .map_err(|e| format!("関数 {name} の引数 {pn}: {e}"))?;
                    let v = self.eval_value(a)?;
                    fenv[0].insert(pn.clone(), v);
                }
                match &f.ret {
                    Some(t) => self.norm.eval_or_opaque(t, &mut fenv)?,
                    None => prim("unit"),
                }
            }
            Expr::Assign(name, x) => {
                let en = self.lookup(name).ok_or(format!("未定義の変数 {name}"))?;
                if !en.mutable {
                    return Err(format!("変数 {name} は mut でない"));
                }
                let t = en.ty.clone();
                self.check(x, &t).map_err(|e| format!("変数 {name}: {e}"))?;
                prim("unit")
            }
            Expr::If(c, then, els) => {
                want(&boolean, &self.expr(c)?, "if の条件")?;
                let tt = self.block(then)?;
                let Some(els) = els else {
                    // else が無い if の値は捨てる
                    return Ok(prim("unit"));
                };
                let et = self.expr(els)?;
                want(&tt, &et, "if の分岐")?;
                if tt == prim("never") { et } else { tt }
            }
            Expr::While(c, body) => {
                want(&boolean, &self.expr(c)?, "while の条件")?;
                self.block(body)?;
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
                    }
                }
                Q::Splice(_, e) => {
                    let t = self.expr(e)?;
                    if t == prim("unit") || t == prim("never") {
                        return Err("${ } の値を差し込めない".into());
                    }
                }
                Q::Rep(inner, _) => self.check_q(inner)?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse;

    fn check(src: &str) -> Result<(), String> {
        super::check(&parse(src)?)
    }

    #[test]
    fn dependent() {
        // 引数の値で戻り値の型が決まる
        let src = r#"
fn T(b: bool) -> Type { if b { int } else { str } }
fn f(b: bool) -> T(b) { if b { 1 } else { "one" } }
fn id(A: Type, x: A) -> A { x }
fn main() {
    let x: int = f(true)
    let s: str = f(false)
    let y: T(1 < 2) = id(int, 3)
    print(x + y)
}
"#;
        check(src).unwrap();
        for (bad, msg) in [
            ("let x: int = f(true)", "let x: int = f(false)"),
            (
                "let y: T(1 < 2) = id(int, 3)",
                "let y: T(1 < 2) = id(int, true)",
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
        let e = check("fn loop(n: int) -> Type { loop(n + 1) } fn main() { let x: loop(0) = 1 }")
            .unwrap_err();
        assert!(e.contains("終わらない"), "{e}");
    }
}
