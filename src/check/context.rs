//! 要求の提供を、隠れた引数（文脈）で渡す書き換え。型検査の後に 1 回だけ行う。
//!
//! 文脈は「要求の操作の名前 → 提供した関数」の並びで、内側の with が足した組を先に探す。
//! - 要求（トレイトか要求の変数）を持つトップレベルの関数と、すべての無名関数は、最後の引数に文脈を受け取る。
//!   関数の値の呼び出しは常に文脈を渡すので、値として使うトップレベルの関数の入口は、文脈を受け取らない関数なら捨てる。
//! - 要求の操作の呼び出し `log(m)` は `#ctx_get(文脈, "T::log")(m, 文脈)` になる。
//! - `with T(値) { 本体 }`（Handle）は、本体の中で腕を無名関数にして文脈に足す。腕は with の外の文脈を写し取る。
//!   Handle は exit の行き先と、整数の範囲外を受け取る腕（IntOverflow）だけを持つ。
//! - 要求の無い所の文脈は空（`Int(0)`）。型検査が提供を確かめているので、空の文脈を引くことは無い。
//! ponytail: 文脈は操作の名前で内側から線形に探す。with が深く重なって遅ければ、型検査で位置を決めて添字で引く
use std::collections::{HashMap, HashSet};

use crate::parser::{Arm, Block, Expr, Fn, Name, NodeId, Program, Stmt};

// トップレベルの関数が文脈を受け取るか。Async は文脈で運ばない
pub fn takes_context(f: &Fn) -> bool {
    f.js.is_none()
        && f.op.is_none()
        && !f.alias
        && f.effects.iter().any(|e| !matches!(e.peel(), Expr::Var(n) if n == "Async"))
}

struct Op {
    // 推論した型の引数の数。呼び出しで省かれていなければ捨てる
    implicit: usize,
    params: usize,
    // メソッドの宣言に with Async がある。腕の Promise を待つ
    waits: bool,
}

struct Pass<'a> {
    takes: HashMap<Name, bool>,
    ops: HashMap<Name, Op>,
    // 利用者が all を定義していなければ組み込みの all
    builtin_all: bool,
    awaits: &'a mut HashSet<NodeId>,
    // 局所変数。トップレベルの関数と同じ名前の呼び出しを、関数の値の呼び出しと見分ける
    scopes: Vec<HashSet<Name>>,
}

pub fn pass(prog: &mut Program) {
    let takes = prog.fns.iter().map(|f| (f.name.clone(), takes_context(f))).collect();
    let ops = prog
        .fns
        .iter()
        .filter(|f| f.op.is_some())
        .map(|f| {
            let waits = f.effects.iter().any(|e| matches!(e.peel(), Expr::Var(n) if n == "Async"));
            (f.name.clone(), Op { implicit: f.implicit, params: f.params.len(), waits })
        })
        .collect();
    let builtin_all = !prog.fns.iter().any(|f| f.name == "all");
    let mut awaits = std::mem::take(&mut prog.async_info.awaits);
    let mut p = Pass { takes, ops, builtin_all, awaits: &mut awaits, scopes: Vec::new() };
    for f in &mut prog.fns {
        if f.js.is_some() || f.op.is_some() || f.alias {
            continue;
        }
        let ctx = takes_context(f).then(fresh);
        p.scopes = vec![f.params.iter().map(|(n, _)| n.clone()).collect()];
        let cur = ctx.clone().map_or(Expr::Int(0), Expr::Var);
        p.block(&mut f.body, &cur);
        if let Some(c) = ctx {
            f.params.push((c, Expr::Var("Context".into())));
        }
    }
    prog.async_info.awaits = awaits;
}

fn fresh() -> Name {
    Name::context(NodeId::default().raw() as usize)
}

impl Pass<'_> {
    fn local(&self, n: &Name) -> bool {
        self.scopes.iter().any(|s| s.contains(n))
    }

    fn scoped<T>(&mut self, names: impl IntoIterator<Item = Name>, f: impl FnOnce(&mut Self) -> T) -> T {
        self.scopes.push(names.into_iter().collect());
        let r = f(self);
        self.scopes.pop();
        r
    }

    fn block(&mut self, b: &mut Block, cur: &Expr) {
        self.scopes.push(HashSet::new());
        for s in &mut b.stmts {
            match s {
                Stmt::Let { name, init, .. } => {
                    self.expr(init, cur, None);
                    self.scopes.last_mut().unwrap().insert(name.clone());
                }
                Stmt::Expr(e) => self.expr(e, cur, None),
            }
        }
        if let Some(t) = &mut b.tail {
            self.expr(t, cur, None);
        }
        self.scopes.pop();
    }

    // id は式を包む Expr::Id の識別子（待つ呼び出しの印）
    fn expr(&mut self, e: &mut Expr, cur: &Expr, id: Option<NodeId>) {
        match e {
            Expr::Id(i, x) => self.expr(x, cur, Some(*i)),
            Expr::At(_, x) | Expr::InModule(_, x) => self.expr(x, cur, id),
            Expr::Unary(_, x) | Expr::Field(x, _) | Expr::Assign(_, x) | Expr::Return(Some(x)) | Expr::Exit(_, x) => {
                self.expr(x, cur, None)
            }
            Expr::Binary(_, a, b) | Expr::Index(a, b) => {
                self.expr(a, cur, None);
                self.expr(b, cur, None);
            }
            Expr::Array(xs) => xs.iter_mut().for_each(|x| self.expr(x, cur, None)),
            Expr::If(c, t, f) => {
                self.expr(c, cur, None);
                self.block(t, cur);
                if let Some(f) = f {
                    self.expr(f, cur, None);
                }
            }
            Expr::While(c, b) => {
                self.expr(c, cur, None);
                self.block(b, cur);
            }
            Expr::Block(b) => self.block(b, cur),
            Expr::For(x, a, b, body) => {
                self.expr(a, cur, None);
                if let Some(b) = b {
                    self.expr(b, cur, None);
                }
                self.scoped([x.clone()], |p| p.block(body, cur));
            }
            Expr::Match(s, arms) => {
                self.expr(s, cur, None);
                for a in arms {
                    self.scoped(a.binds.clone(), |p| p.expr(&mut a.body, cur, None));
                }
            }
            Expr::Lam(params, _, _, body) => {
                let c = fresh();
                let names = params.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
                self.scoped(names, |p| p.block(body, &Expr::Var(c.clone())));
                params.push((c, None));
            }
            Expr::Apply(f, args) => {
                self.expr(f, cur, None);
                args.iter_mut().for_each(|x| self.expr(x, cur, None));
                args.push(cur.clone());
            }
            Expr::Call(n, args) => {
                args.iter_mut().for_each(|x| self.expr(x, cur, None));
                if n == "#resume" {
                    *e = args.pop().expect("#resume の引数は1個");
                } else if self.local(n) {
                    args.push(cur.clone());
                } else if let Some(op) = self.ops.get(n) {
                    let skip = args.len() - (op.params - op.implicit);
                    let mut args: Vec<Expr> = args.drain(skip..).collect();
                    args.push(cur.clone());
                    let arm = Expr::Call("#ctx_get".into(), vec![cur.clone(), Expr::Str(n.to_string())]);
                    if op.waits && let Some(id) = id {
                        self.awaits.insert(id);
                    }
                    *e = Expr::Apply(Box::new(arm), args);
                } else if self.takes.get(n).copied().unwrap_or(false) {
                    args.push(cur.clone());
                } else if n == "all" && self.builtin_all {
                    args.push(cur.clone());
                }
            }
            Expr::Handle(_, body, arms) => self.handle(body, arms, cur),
            _ => {}
        }
    }

    // with の本体の前に、腕を無名関数にして文脈に足す。腕は with の外の文脈 cur を写し取る（腕の中の要求は外が提供する）
    fn handle(&mut self, body: &mut Expr, arms: &mut Vec<Arm>, cur: &Expr) {
        let inner = fresh();
        let mut stmts = Vec::new();
        let mut ctx = cur.clone();
        let mut kept = Vec::new();
        for mut a in std::mem::take(arms) {
            if crate::backend::is_integer_overflow_arm(&a.ctor) {
                let mut overflow = a.clone();
                self.scoped(overflow.binds.clone(), |p| p.expr(&mut overflow.body, cur, None));
                kept.push(overflow);
            }
            let b = match std::mem::replace(&mut a.body, Expr::Int(0)) {
                Expr::Block(b) => b,
                x => Block { id: NodeId::default(), stmts: vec![], tail: Some(Box::new(x)) },
            };
            let unused = fresh();
            let mut lam = Expr::Lam(a.binds.iter().map(|n| (n.clone(), None)).collect(), None, None, b);
            // 腕の本体は外の文脈を使う。無名関数として受け取る文脈は使わない
            if let Expr::Lam(params, _, _, b) = &mut lam {
                self.scoped(a.binds.clone(), |p| p.block(b, cur));
                params.push((unused, None));
            }
            let tmp = Name::temporary("arm", NodeId::default().raw() as usize);
            stmts.push(Stmt::Let { name: tmp.clone(), mutable: false, ty: None, init: lam });
            ctx = Expr::Call("#ctx_put".into(), vec![ctx, Expr::Str(a.ctor.to_string()), Expr::Var(tmp)]);
        }
        stmts.push(Stmt::Let { name: inner.clone(), mutable: false, ty: None, init: ctx });
        let mut b = std::mem::replace(body, Expr::Int(0));
        self.scoped([inner.clone()], |p| p.expr(&mut b, &Expr::Var(inner.clone()), None));
        *body = Expr::Block(Block { id: NodeId::default(), stmts, tail: Some(Box::new(b)) });
        *arms = kept;
    }
}
