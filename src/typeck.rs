use crate::parser::{Block, Expr, Fn, Stmt, Type};
use std::collections::HashMap;

type R<T> = Result<T, String>;

// ponytail: エラーに行番号が無い。AST にスパンを持たせる時に追加する
struct Ck<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    // (型, 可変か) のスコープ列
    scopes: Vec<HashMap<String, (Type, bool)>>,
    ret: Type,
}

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
    };
    for f in fns {
        if f.name == "print" || ck.fns.insert(&f.name, f).is_some() {
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
        want(f.ret, t, &format!("関数 {} の戻り値", f.name))?;
    }
    Ok(())
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
}
