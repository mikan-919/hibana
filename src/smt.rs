// 型検査の証明器で示せなかった条件を、Z3 に問い合わせる
use crate::typeck::{Neu, V};
use easy_smt::{Context, ContextBuilder, Response, SExpr};
use std::collections::HashMap;
use std::io;

// ponytail: int を上限の無い整数として扱う。オーバーフローは実行時に検査している。厳密にするなら BitVec 64 にする
pub struct Smt {
    // None は未起動、Some(None) は z3 が無い
    ctx: Option<Option<Context>>,
}

impl Smt {
    pub fn new() -> Self {
        Smt { ctx: None }
    }

    // facts がすべて真なら goal も真か。z3 が無い・答えが出ない時は false
    pub fn prove(&mut self, facts: &[V], goal: &V) -> bool {
        let ctx = self
            .ctx
            .get_or_insert_with(|| ContextBuilder::new().with_z3_defaults().build().ok());
        let Some(ctx) = ctx else { return false };
        if ctx.push().is_err() {
            return false;
        }
        let r = query(ctx, facts, goal);
        // pop に失敗しても、次の問い合わせで宣言が重複するだけなので結果は使う
        let _ = ctx.pop();
        r.unwrap_or(false)
    }
}

// facts を仮定し、goal の否定が充足不能なら証明できたことになる
fn query(ctx: &mut Context, facts: &[V], goal: &V) -> io::Result<bool> {
    let mut tr = Tr {
        ctx,
        atoms: HashMap::new(),
    };
    for f in facts {
        let s = tr.term(f, false)?;
        tr.ctx.assert(s)?;
    }
    let g = tr.term(goal, false)?;
    let not_g = tr.ctx.list(vec![tr.ctx.atom("not"), g]);
    tr.ctx.assert(not_g)?;
    Ok(tr.ctx.check()? == Response::Unsat)
}

// V を SMT-LIB の項に訳す。訳せない項（関数呼び出し・変数など）は、同じ項ごとに1個の定数にする
struct Tr<'a> {
    ctx: &'a mut Context,
    atoms: HashMap<(String, bool), SExpr>,
}

// 項が int か bool か分かれば返す
fn sort_of(v: &V) -> Option<bool> {
    match v {
        V::Int(_) => Some(true),
        V::Bool(_) => Some(false),
        V::Neu(n) => match &**n {
            Neu::Un("-", _) | Neu::Bin("+" | "-" | "*", ..) => Some(true),
            Neu::Un("!", _) | Neu::Bin("<" | "<=" | "==" | "!=" | "&&" | "||", ..) => Some(false),
            _ => None,
        },
        _ => None,
    }
}

impl Tr<'_> {
    fn app(&self, f: &str, args: Vec<SExpr>) -> SExpr {
        let mut l = vec![self.ctx.atom(f)];
        l.extend(args);
        self.ctx.list(l)
    }

    // int なら Int、そうでなければ Bool の項にする
    fn term(&mut self, v: &V, int: bool) -> io::Result<SExpr> {
        match (v, int) {
            (V::Int(n), true) => {
                let a = self.ctx.atom(n.unsigned_abs().to_string());
                return Ok(if *n < 0 { self.app("-", vec![a]) } else { a });
            }
            (V::Bool(b), false) => {
                return Ok(if *b {
                    self.ctx.true_()
                } else {
                    self.ctx.false_()
                });
            }
            (V::Neu(n), _) => {
                let r = match (&**n, int) {
                    (Neu::Un("-", x), true) => Some(("-", vec![self.term(x, true)?])),
                    (Neu::Un("!", x), false) => Some(("not", vec![self.term(x, false)?])),
                    (Neu::Bin(op @ ("+" | "-" | "*"), a, b), true) => {
                        Some((*op, vec![self.term(a, true)?, self.term(b, true)?]))
                    }
                    (Neu::Bin(op @ ("<" | "<="), a, b), false) => {
                        Some((*op, vec![self.term(a, true)?, self.term(b, true)?]))
                    }
                    (Neu::Bin(op @ ("&&" | "||"), a, b), false) => {
                        let f = if *op == "&&" { "and" } else { "or" };
                        Some((f, vec![self.term(a, false)?, self.term(b, false)?]))
                    }
                    (Neu::Bin(op @ ("==" | "!="), a, b), false) => {
                        let s = sort_of(a).or(sort_of(b)).unwrap_or(true);
                        let f = if *op == "==" { "=" } else { "distinct" };
                        Some((f, vec![self.term(a, s)?, self.term(b, s)?]))
                    }
                    _ => None,
                };
                if let Some((f, args)) = r {
                    return Ok(self.app(f, args));
                }
            }
            _ => {}
        }
        let key = (format!("{v:?}"), int);
        if let Some(a) = self.atoms.get(&key) {
            return Ok(*a);
        }
        let sort = if int {
            self.ctx.int_sort()
        } else {
            self.ctx.bool_sort()
        };
        let a = self
            .ctx
            .declare_const(format!("t{}", self.atoms.len()), sort)?;
        self.atoms.insert(key, a);
        Ok(a)
    }
}
