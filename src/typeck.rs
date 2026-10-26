use crate::meta::Q;
use crate::parser::{Block, Expr, Fn, Stmt, Type};
use std::collections::HashMap;

type R<T> = Result<T, String>;

// ponytail: エラーに行番号が無い。AST にスパンを持たせる時に追加する
struct Ck<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    // (型, 可変か) のスコープ列
    scopes: Vec<HashMap<String, (Type, bool)>>,
    ret: Type,
    // マクロ本体（コンパイル時に実行するコード）の中か
    in_macro: bool,
}

// マクロ本体の中でだけ使える組み込み関数 (名前, 引数, 戻り値)
const SYN_FNS: &[(&str, &[Type], Type)] = &[
    ("len", &[Type::Syn], Type::Int),
    ("at", &[Type::Syn, Type::Int], Type::Syn),
    ("is_null", &[Type::Syn], Type::Bool),
    ("int_of", &[Type::Syn], Type::Int),
    ("str_of", &[Type::Syn], Type::Str),
    ("error", &[Type::Str], Type::Never),
];

// Never はどの型とも一致する
fn same(a: Type, b: Type) -> bool {
    a == b || a == Type::Never || b == Type::Never
}

fn want(e: Type, got: Type, what: &str) -> R<()> {
    if same(e, got) {
        Ok(())
    } else {
        Err(format!("{what}: {e:?} が必要だが {got:?}"))
    }
}

pub fn check(fns: &[Fn]) -> R<()> {
    let mut ck = Ck {
        fns: HashMap::new(),
        scopes: Vec::new(),
        ret: Type::Unit,
        in_macro: false,
    };
    for f in fns {
        if f.name == "print"
            || SYN_FNS.iter().any(|s| s.0 == f.name)
            || ck.fns.insert(&f.name, f).is_some()
        {
            return Err(format!("関数 {} が重複している", f.name));
        }
    }
    match ck.fns.get("main") {
        Some(f) if f.params.is_empty() => {}
        _ => return Err("引数なしの main 関数が必要".into()),
    }
    for f in fns {
        ck.ret = f.ret;
        ck.scopes = vec![
            f.params
                .iter()
                .map(|(n, t)| (n.clone(), (*t, false)))
                .collect(),
        ];
        let t = ck.block(&f.body)?;
        // 戻り値が unit の関数は最後の式の値を捨てる
        if f.ret != Type::Unit {
            want(f.ret, t, &format!("関数 {} の戻り値", f.name))?;
        }
    }
    Ok(())
}

// マクロ本体を、束縛名を syn 型の変数として検査する。値は syn 型でなければならない
pub fn check_macro(b: &Block, params: &[String]) -> R<()> {
    let mut ck = Ck {
        fns: HashMap::new(),
        scopes: vec![
            params
                .iter()
                .map(|n| (n.clone(), (Type::Syn, false)))
                .collect(),
        ],
        ret: Type::Syn,
        in_macro: true,
    };
    let t = ck.block(b)?;
    want(Type::Syn, t, "マクロ本体の値")
}

impl Ck<'_> {
    fn lookup(&self, name: &str) -> R<(Type, bool)> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name).copied())
            .ok_or(format!("未定義の変数 {name}"))
    }

    fn block(&mut self, b: &Block) -> R<Type> {
        self.scopes.push(HashMap::new());
        let r = self.block_inner(b);
        self.scopes.pop();
        r
    }

    fn block_inner(&mut self, b: &Block) -> R<Type> {
        let mut diverges = false;
        for s in &b.stmts {
            match s {
                Stmt::Let {
                    name,
                    mutable,
                    ty,
                    init,
                } => {
                    let t = self.expr(init)?;
                    if let Some(ty) = ty {
                        want(*ty, t, &format!("変数 {name}"))?;
                    }
                    let t = ty.unwrap_or(t);
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .insert(name.clone(), (t, *mutable));
                }
                Stmt::Expr(e) => diverges |= self.expr(e)? == Type::Never,
            }
        }
        match &b.tail {
            Some(e) => self.expr(e),
            None if diverges => Ok(Type::Never),
            None => Ok(Type::Unit),
        }
    }

    fn expr(&mut self, e: &Expr) -> R<Type> {
        use Type::*;
        Ok(match e {
            Expr::Int(_) => Int,
            Expr::Bool(_) => Bool,
            Expr::Str(_) => Str,
            Expr::Field(x, _) => {
                want(Syn, self.expr(x)?, "フィールド参照")?;
                Syn
            }
            Expr::Method(..) => return Err("メソッドは未対応".into()),
            Expr::Quote(q) => {
                if !self.in_macro {
                    return Err("quote はマクロ本体の中でだけ使える".into());
                }
                self.check_q(q)?;
                Syn
            }
            Expr::Macro(n, _) => return Err(format!("マクロ {n} に展開コードが無い")),
            Expr::Var(n) => self.lookup(n)?.0,
            Expr::Unary(op, x) => {
                let t = if *op == "-" { Int } else { Bool };
                want(t, self.expr(x)?, op)?;
                t
            }
            Expr::Binary(op, l, r) => {
                let (lt, rt) = (self.expr(l)?, self.expr(r)?);
                match *op {
                    "+" | "-" | "*" | "/" | "%" | "<" | "<=" | ">" | ">=" => {
                        want(Int, lt, op)?;
                        want(Int, rt, op)?;
                        if matches!(*op, "+" | "-" | "*" | "/" | "%") {
                            Int
                        } else {
                            Bool
                        }
                    }
                    "&&" | "||" => {
                        want(Bool, lt, op)?;
                        want(Bool, rt, op)?;
                        Bool
                    }
                    _ => {
                        // == と !=
                        want(lt, rt, op)?;
                        if lt == Syn {
                            return Err("syn は比較できない".into());
                        }
                        Bool
                    }
                }
            }
            Expr::Call(name, args) if name == "print" => {
                if args.len() != 1 {
                    return Err("print の引数は1個".into());
                }
                self.expr(&args[0])?;
                Unit
            }
            Expr::Call(name, args) if self.in_macro && SYN_FNS.iter().any(|s| s.0 == name) => {
                let (_, params, ret) = SYN_FNS.iter().find(|s| s.0 == name).unwrap();
                if params.len() != args.len() {
                    return Err(format!("{name} の引数は{}個", params.len()));
                }
                for (p, a) in params.iter().zip(args) {
                    want(*p, self.expr(a)?, name)?;
                }
                *ret
            }
            Expr::Call(name, args) => {
                let f = *self
                    .fns
                    .get(name.as_str())
                    .ok_or(format!("未定義の関数 {name}"))?;
                let (params, ret) = (&f.params, f.ret);
                if params.len() != args.len() {
                    return Err(format!("関数 {name} の引数は{}個", params.len()));
                }
                for ((pn, pt), a) in params.iter().zip(args) {
                    want(*pt, self.expr(a)?, &format!("関数 {name} の引数 {pn}"))?;
                }
                ret
            }
            Expr::Assign(name, x) => {
                let (t, mutable) = self.lookup(name)?;
                if !mutable {
                    return Err(format!("変数 {name} は mut でない"));
                }
                want(t, self.expr(x)?, &format!("変数 {name}"))?;
                Unit
            }
            Expr::If(c, then, els) => {
                want(Bool, self.expr(c)?, "if の条件")?;
                let tt = self.block(then)?;
                let et = match els {
                    Some(x) => self.expr(x)?,
                    None => Unit,
                };
                if els.is_none() {
                    // else が無い if の値は捨てる
                    return Ok(Unit);
                }
                want(tt, et, "if の分岐")?;
                if tt == Never { et } else { tt }
            }
            Expr::While(c, body) => {
                want(Bool, self.expr(c)?, "while の条件")?;
                self.block(body)?;
                Unit
            }
            Expr::Block(b) => self.block(b)?,
            Expr::Return(x) => {
                let t = match x {
                    Some(x) => self.expr(x)?,
                    None => Unit,
                };
                want(self.ret, t, "return")?;
                Never
            }
        })
    }

    fn check_q(&mut self, q: &[Q]) -> R<()> {
        for x in q {
            match x {
                Q::Tok(_) => {}
                Q::Var { path, .. } => {
                    let t = self.lookup(&path[0])?.0;
                    if path.len() > 1 {
                        want(Type::Syn, t, &format!("${}", path.join(".")))?;
                    }
                }
                Q::Splice(_, e) => {
                    if matches!(self.expr(e)?, Type::Unit | Type::Never) {
                        return Err("${ } の値を差し込めない".into());
                    }
                }
                Q::Rep(inner, _) => self.check_q(inner)?,
            }
        }
        Ok(())
    }
}
