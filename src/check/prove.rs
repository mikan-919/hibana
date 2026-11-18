//! 細分化型の項・評価器・事実の照合。
use crate::parser::Name;
use super::typeck::{bound, neu, prim, ref_t, PiV, V, PRIMS, SYN_FNS, MIN_SAFE_INT, MAX_SAFE_INT};
use crate::error::{Error, R};
use crate::parser::{Ctor, Data, Expr, Fn};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, PartialEq)]
pub(crate) enum Term {
    // 引数など、値の分からない変数。番号で区別する。0 は細分化型の条件の変数
    Var(usize, Name),
    Un(&'static str, V),
    Bin(&'static str, V, V),
    // 長さなどの記号的な呼び出し
    Call(Name, Vec<V>),
    // 値の分からない構造体のフィールド
    Proj(V, Name),
    // 型の計算で扱えない式（while・代入・put など）。どの値とも等しくない
    Opaque(usize),
    // number の値。整数用の証明規則を適用しないための印
    Num(V),
}

pub(super) fn num(v: V) -> V {
    match v {
        V::Int(n) => V::Num(n as f64),
        V::Num(_) => v,
        V::Neu(ref n) if matches!(&**n, Term::Num(_)) => v,
        _ => neu(Term::Num(v)),
    }
}

fn has_num(v: &V) -> bool {
    match v {
        V::Num(_) => true,
        V::Neu(n) => match &**n {
            Term::Num(_) => true,
            Term::Un(_, v) | Term::Proj(v, _) => has_num(v),
            Term::Bin(_, a, b) => has_num(a) || has_num(b),
            Term::Call(_, args) => args.iter().any(has_num),
            _ => false,
        },
        _ => false,
    }
}

// 項と定数の和に分ける。定数だけなら項は 0
fn lin(v: &V) -> (V, i64) {
    match v {
        V::Int(c) => (V::Int(0), *c),
        V::Neu(m) => match &**m {
            Term::Bin("+", x, V::Int(c)) => (x.clone(), *c),
            Term::Bin("-", x, V::Int(c)) => (x.clone(), c.saturating_neg()),
            _ => (v.clone(), 0),
        },
        _ => (v.clone(), 0),
    }
}

// 比較を a <= b + k の形に直す。a < b は a <= b - 1。両辺の定数は k に寄せる（2 <= n は 0 <= n - 2）
fn le_form(v: &V) -> Option<(V, V, i64)> {
    let V::Neu(n) = v else { return None };
    let (a, b, k): (&V, &V, i64) = match &**n {
        Term::Bin("<=", a, b) => (a, b, 0),
        Term::Bin("<", a, b) => (a, b, -1),
        _ => return None,
    };
    let ((ta, ca), (tb, cb)) = (lin(a), lin(b));
    Some((ta, tb, k.saturating_add(cb).saturating_sub(ca)))
}

// 項 from を to に置き換え、計算できる所を計算し直す
pub(super) fn replace(v: &V, from: &V, to: &V) -> V {
    if v == from {
        return to.clone();
    }
    let V::Neu(n) = v else { return v.clone() };
    match &**n {
        Term::Bin(op, a, b) => bin(op, replace(a, from, to), replace(b, from, to)),
        Term::Un(op, x) => un(op, replace(x, from, to)),
        _ => v.clone(),
    }
}

// 項の中の len(変数 id) を t に置き換える（足し算・引き算の中だけ）
pub(super) fn replace_len(v: &V, id: usize, t: &V) -> V {
    if let V::Neu(n) = v {
        match &**n {
            Term::Call(f, a)
                if f == "len"
                    && matches!(&a[..], [V::Neu(x)] if matches!(&**x, Term::Var(i, _) if *i == id)) =>
            {
                return t.clone();
            }
            Term::Bin(op, a, b) if matches!(*op, "+" | "-") => {
                return bin(op, replace_len(a, id, t), replace_len(b, id, t));
            }
            _ => {}
        }
    }
    v.clone()
}

pub(super) fn prove_facts<'a>(goal: &V, facts: impl Iterator<Item = &'a V> + Clone) -> bool {
    if let V::Neu(n) = goal {
        match &**n {
            Term::Bin("&&", a, b) => return prove_facts(a, facts.clone()) && prove_facts(b, facts.clone()),
            Term::Bin("||", a, b) => return prove_facts(a, facts.clone()) || prove_facts(b, facts.clone()),
            _ => {}
        }
    }
    let g_le = le_form(goal);
    // a <= a + k は k >= 0 なら成り立つ
    if let Some((a, b, k)) = &g_le
        && a == b
        && *k >= 0
    {
        return true;
    }
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
            (Term::Bin("<", a, b), Term::Bin("!=", c, d)) => {
                (a == c && b == d) || (a == d && b == c)
            }
            _ => false,
        }
    };
    *goal == V::Bool(true) || facts.clone().any(implies)
}

// #num の印を外して同じ数値の項を照合する。
fn number_term(v: &V) -> &V {
    if let V::Neu(n) = v && let Term::Num(v) = &**n { number_term(v) } else { v }
}

fn constant(v: &V) -> Option<f64> {
    match number_term(v) { V::Int(n) => Some(*n as f64), V::Num(n) if n.is_finite() => Some(*n), _ => None }
}

// 厳密不等式も閉区間へ弱める。端点は最近接・偶数丸めの単調性で伝播する。
fn interval(v: &V, facts: &[&V]) -> Option<(f64, f64)> {
    let v = number_term(v);
    if let Some(c) = constant(v) { return Some((c, c)); }
    let mut lo: Option<f64> = None;
    let mut hi: Option<f64> = None;
    for f in facts {
        if let V::Neu(n) = f && let Term::Bin(op, a, b) = &**n {
            if number_term(a) == v && let Some(c) = constant(b) {
                if matches!(*op, "<" | "<=" | "==") { hi = Some(hi.map_or(c, |h| h.min(c))); }
                if *op == "==" { lo = Some(lo.map_or(c, |l| l.max(c))); }
            }
            if number_term(b) == v && let Some(c) = constant(a) {
                if matches!(*op, "<" | "<=" | "==") { lo = Some(lo.map_or(c, |l| l.max(c))); }
                if *op == "==" { hi = Some(hi.map_or(c, |h| h.min(c))); }
            }
        }
    }
    let propagated = match v {
        V::Neu(n) => match &**n {
            Term::Un("-", x) => interval(x, facts).map(|(a,b)| (-b,-a)),
            Term::Bin(op @ ("+" | "-" | "*"), x, y) => {
                match (interval(x, facts), interval(y, facts)) {
                    (Some((a,b)), Some((c,d))) => Some(match *op {
                        "+" => (a+c,b+d), "-" => (a-d,b-c),
                        _ => { let corners = [a*c,a*d,b*c,b*d];
                            if !corners.iter().all(|c| c.is_finite()) { return None; }
                            (corners.into_iter().fold(f64::INFINITY, f64::min), corners.into_iter().fold(f64::NEG_INFINITY, f64::max))
                        }
                    }),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    };
    if let Some((a,b)) = propagated && a.is_finite() && b.is_finite() {
        lo = Some(lo.map_or(a, |l| l.max(a)));
        hi = Some(hi.map_or(b, |h| h.min(b)));
    }
    match (lo,hi) { (Some(a),Some(b)) if a <= b => Some((a,b)), _ => None }
}

fn sign(v: &V, facts: &[&V], positive: bool) -> bool {
    if let Some(c) = constant(v) { return if positive { c > 0.0 } else { c >= 0.0 }; }
    for f in facts {
        if let V::Neu(n) = f && let Term::Bin(op, a, b) = &**n
            && number_term(b) == number_term(v) && let Some(c) = constant(a)
            && ((matches!(*op, "<" | "<=" | "==") && c > 0.0)
                || (c == 0.0 && (*op == "<" || (!positive && matches!(*op, "<=" | "=="))))) { return true; }
        if let V::Neu(n) = f && let Term::Bin("==", a, b) = &**n
            && number_term(a) == number_term(v) && let Some(c) = constant(b)
            && (if positive { c > 0.0 } else { c >= 0.0 }) { return true; }
    }
    if let Some((lo,_)) = interval(v, facts) && (if positive { lo > 0.0 } else { lo >= 0.0 }) { return true; }
    if !positive && let V::Neu(n) = number_term(v) {
        match &**n {
            Term::Bin("+" | "*", a, b) => return sign(a, facts, false) && sign(b, facts, false),
            Term::Bin("/", a, b) => return sign(a, facts, false) && sign(b, facts, true),
            Term::Call(f, _) if f == "num_sqrt" => return true,
            _ => {}
        }
    }
    false
}

fn strict_shift(a: &V, b: &V, facts: &[&V]) -> bool {
    let shifted = |v: &V, shifted: &V, subtract: bool| {
        let V::Neu(n) = number_term(shifted) else { return false };
        let Term::Bin(op, x, c) = &**n else { return false };
        if *op != (if subtract { "-" } else { "+" }) || number_term(x) != number_term(v) { return false; }
        let Some(c) = constant(c).filter(|c| *c > 0.0) else { return false };
        let Some((lo,hi)) = interval(v, facts) else { return false };
        let m = lo.abs().max(hi.abs());
        let delta = if subtract { -c } else { c };
        (lo + delta).is_finite() && (hi + delta).is_finite() && 2.0 * (m.next_up() - m) <= c
    };
    shifted(a, b, false) || shifted(b, a, true)
}

fn prove_number(goal: &V, facts: &[&V]) -> bool {
    if *goal == V::Bool(true) || facts.contains(&goal) { return true; }
    let V::Neu(n) = goal else { return false };
    match &**n {
        Term::Bin("&&", a, b) => prove_number(a, facts) && prove_number(b, facts),
        Term::Bin("||", a, b) => prove_number(a, facts) || prove_number(b, facts),
        Term::Bin(op, a, b) => {
            if *op == "<=" && constant(a) == Some(0.0) && sign(b, facts, false) { return true; }
            if *op == "<" && constant(a) == Some(0.0) && sign(b, facts, true) { return true; }
            if *op == "<" && strict_shift(a, b, facts) { return true; }
            if *op == "<=" && number_term(a) == number_term(b) { return true; }
            match (interval(a, facts), interval(b, facts)) {
                (Some((al,ah)),Some((bl,bh))) => match *op {
                    "<=" => ah <= bl, "<" => ah < bl, "!=" => ah < bl || bh < al,
                    "==" => al == ah && bl == bh && al == bl, _ => false,
                },
                _ => false,
            }
        }
        _ => false,
    }
}

// 事実との照合と定数の代入で示し、残りを Z3 に問い合わせる。
pub(super) fn prove<'a>(
    goal: &V,
    facts: impl Iterator<Item = &'a V> + Clone,
    smt: &mut super::smt::Smt,
) -> bool {
    // 浮動小数点数は専用の規則で扱う。整数のゴールからも浮動小数点数の事実を除く。
    if has_num(goal) {
        return prove_number(goal, &facts.collect::<Vec<_>>());
    }
    let integer_facts: Vec<_> = facts.filter(|f| !has_num(f)).collect();
    let facts = integer_facts.into_iter();
    if prove_facts(goal, facts.clone()) {
        return true;
    }
    let eqs: Vec<_> = facts
        .clone()
        .filter_map(|f| match f {
            V::Neu(n) => match &**n {
                Term::Bin("==", x, c @ V::Int(_)) | Term::Bin("==", c @ V::Int(_), x) => {
                    Some((x, c))
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    if !eqs.is_empty() {
        let g = eqs.iter().fold(goal.clone(), |g, (x, c)| replace(&g, x, c));
        if g == V::Bool(true) || prove_facts(&g, facts.clone()) {
            return true;
        }
    }
    smt.prove(&facts.cloned().collect::<Vec<_>>(), goal)
}

pub(super) fn un(op: &'static str, v: V) -> V {
    if let V::Neu(n) = &v
        && let (true, Term::Bin(o, a, b)) = (op == "!", &**n)
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
            return bin(o, a.clone(), b.clone());
        }
    }
    match (op, v) {
        ("-", V::Int(n)) if (MIN_SAFE_INT..=MAX_SAFE_INT).contains(&n) => V::Int(-n),
        ("-", V::Num(n)) => V::Num(-n),
        ("!", V::Bool(b)) => V::Bool(!b),
        (op, v) => neu(Term::Un(op, v)),
    }
}

pub(super) fn bin(op: &'static str, a: V, b: V) -> V {
    use V::*;
    let numeric = |v: &V| match v {
        Num(_) => true,
        Neu(n) => matches!(&**n, Term::Num(_) | Term::Un("-", _) | Term::Bin("+" | "-" | "*" | "/" | "%" | "**", ..)) && has_num(v),
        _ => false,
    };
    let (a, b) = if numeric(&a) || numeric(&b) { (num(a), num(b)) } else { (a, b) };
    let r = match (op, &a, &b) {
        (op, Num(x), Num(y)) => match op {
            "+" => Some(Num(x + y)), "-" => Some(Num(x - y)), "*" => Some(Num(x * y)),
            "/" if *y != 0.0 => Some(Num(x / y)), "%" if *y != 0.0 => Some(Num(x % y)), "**" => Some(Num(crate::eval::num_pow(*x, *y))),
            "<" => Some(Bool(x < y)), "<=" => Some(Bool(x <= y)), ">" => Some(Bool(x > y)), ">=" => Some(Bool(x >= y)),
            "==" => Some(Bool(x == y)), "!=" => Some(Bool(x != y)), _ => None,
        },
        ("+", Int(x), Int(y)) => x.checked_add(*y).map(Int),
        ("+", Str(x), Str(y)) => Some(Str(format!("{x}{y}").into())),
        ("-", Int(x), Int(y)) => x.checked_sub(*y).map(Int),
        ("*", Int(x), Int(y)) => x.checked_mul(*y).map(Int),
        ("/", Int(x), Int(y)) => x.checked_div(*y).map(Int),
        ("%", Int(x), Int(y)) => x.checked_rem(*y).map(Int),
        ("**", Int(x), Int(y)) => u32::try_from(*y).ok().and_then(|e| x.checked_pow(e)).map(Int),
        ("<", Int(x), Int(y)) => Some(Bool(x < y)),
        ("<=", Int(x), Int(y)) => Some(Bool(x <= y)),
        (">", Int(x), Int(y)) => Some(Bool(x > y)),
        (">=", Int(x), Int(y)) => Some(Bool(x >= y)),
        // 片方が分かれば簡約する（true && p は p）
        ("&&", Bool(true), _) => Some(b.clone()),
        ("&&", _, Bool(true)) => Some(a.clone()),
        ("&&", Bool(false), _) | ("&&", _, Bool(false)) => Some(Bool(false)),
        ("||", Bool(false), _) => Some(b.clone()),
        ("||", _, Bool(false)) => Some(a.clone()),
        ("||", Bool(true), _) | ("||", _, Bool(true)) => Some(Bool(true)),
        // 同じ項は値が分からなくても等しい
        ("==", _, _) if a == b => Some(Bool(true)),
        ("!=", _, _) if a == b => Some(Bool(false)),
        ("==" | "!=", x, y) if !matches!(x, Neu(_)) && !matches!(y, Neu(_)) => {
            Some(Bool((a == b) == (op == "==")))
        }
        _ => None,
    };
    if let Some(r) = r {
        let valid = match &r { Int(n) => (MIN_SAFE_INT..=MAX_SAFE_INT).contains(n), Num(n) => n.is_finite(), _ => true };
        if valid { return r; }
    }
    // > と >= は向きを揃えて < と <= にする
    match op {
        ">" => neu(Term::Bin("<", b, a)),
        ">=" => neu(Term::Bin("<=", b, a)),
        _ => neu(Term::Bin(op, a, b)),
    }
}

pub(super) fn len(v: V) -> V {
    match &v {
        V::Arr(xs) => V::Int(xs.len() as i64),
        V::Str(s) => V::Int(s.chars().count() as i64),
        V::Neu(n) => match &**n {
            // push は 1 つ増やし、set_at は変えない
            Term::Call(f, a) if f == "push" && a.len() == 2 => {
                bin("+", len(a[0].clone()), V::Int(1))
            }
            Term::Call(f, a) if f == "set_at" && a.len() == 3 => len(a[0].clone()),
            _ => neu(Term::Call("len".into(), vec![v.clone()])),
        },
        _ => neu(Term::Call("len".into(), vec![v])),
    }
}

pub(super) enum Halt {
    Stuck,
    Err(Error),
}

pub(super) type H<T> = Result<T, Halt>;

// 型の計算と、細分化型の条件の評価をする評価器
// ponytail: 展開は燃料と深さに上限があり、超えると記号として残す
pub(super) struct Eval {
    pub(super) fns: HashMap<Name, Fn>,
    pub(super) datas: HashMap<Name, Data>,
    // 構築子名から (型, 構築子)
    pub(super) ctors: HashMap<Name, (Data, Ctor)>,
    pub(super) fuel: usize,
    // 展開の入れ子の深さ。スタックあふれを防ぐ
    pub(super) depth: usize,
    pub(super) next_id: usize,
}

pub(super) type Env = Vec<HashMap<Name, V>>;

impl Eval {
    pub(super) fn new() -> Self {
        Eval {
            fns: HashMap::new(),
            datas: HashMap::new(),
            ctors: HashMap::new(),
            fuel: 100_000,
            depth: 0,
            next_id: 0,
        }
    }

    pub(super) fn fresh(&mut self) -> usize {
        self.next_id += 1;
        self.next_id
    }

    pub(super) fn opaque(&mut self) -> V {
        let id = self.fresh();
        neu(Term::Opaque(id))
    }

    // 止まった計算は、どの値とも等しくない項にする
    pub(super) fn eval_or_opaque(&mut self, e: &Expr, env: &mut Env) -> R<V> {
        match self.eval(e, env) {
            Ok(v) => Ok(v),
            Err(Halt::Stuck) => Ok(self.opaque()),
            Err(Halt::Err(e)) => Err(e),
        }
    }

    // 構造体のフィールド。値が分からなければ x.f という項にする
    pub(super) fn proj(&self, x: V, f: &Name) -> V {
        if let V::Data(d) = &x
            && let Some((_, c)) = self.ctors.get(&d.0)
            && let Some(i) = c.fields.iter().position(|(n, _)| n == f)
        {
            return d.1[i].clone();
        }
        neu(Term::Proj(x, f.clone()))
    }

    // 関数型の式を評価する。型は引数の値に依存しない
    pub(super) fn eval_pi(
        &mut self,
        binders: &[(Name, Expr)],
        ret: &Expr,
        effs: &[Expr],
        env: &mut Env,
    ) -> H<V> {
        let mut doms = Vec::new();
        for (_, t) in binders {
            doms.push(self.eval(t, env)?);
        }
        let cod = self.eval(ret, env)?;
        let mut es = Vec::new();
        for e in effs {
            es.push(match e.peel() {
                Expr::Var(n) if n == "Async" => V::DataT(Rc::new(("Async".into(), vec![]))),
                e => self.eval(e, env)?,
            });
        }
        Ok(V::Pi(Rc::new(PiV {
            doms,
            cod,
            effs: es,
        })))
    }

    // 許可した呼び出しだけを記号計算し、実行時関数は呼び出しごとに区別する
    pub(super) fn call(&mut self, name: &Name, args: Vec<V>) -> R<V> {
        match (name.as_str(), args.as_slice()) {
            ("len", [v]) => return Ok(len(v.clone())),
            ("Vec", [a]) => return Ok(V::VecT(Rc::new(a.clone()))),
            ("Ref", [a]) => return Ok(ref_t(a.clone())),
            ("Async" | "Task", [a]) => return Ok(V::DataT(Rc::new((name.into(), vec![a.clone()])))),
            _ if self.ctors.contains_key(name) => return Ok(V::Data(Rc::new((name.into(), args)))),
            _ if self.datas.contains_key(name) => {
                return Ok(V::DataT(Rc::new((name.into(), args))));
            }
            ("num_floor" | "num_sqrt" | "to_str" | "split" | "starts_with" | "slice"
                | "chars" | "code" | "char_from_codepoint" | "push" | "set_at" | "[]", _) => {
                return Ok(neu(Term::Call(name.into(), args)));
            }
            _ => {}
        }
        // 実行時関数は展開せず、型別名だけを展開する
        let Some(f) = self.fns.get(name).filter(|f| f.alias).cloned() else {
            return Ok(self.opaque());
        };
        // 制約の実装を省いた呼び出しは展開しない
        if f.insts > 0 && args.len() < f.params.len() - f.implicit {
            return Ok(neu(Term::Call(name.into(), args)));
        }
        // 省かれたジェネリック引数は、値の計算には使わないので () で埋める
        let args = if args.len() + f.implicit == f.params.len() {
            let mut full = vec![V::Unit; f.implicit];
            full.extend(args);
            full
        } else {
            args
        };
        if self.fuel == 0 || self.depth >= 64 {
            return Ok(neu(Term::Call(name.into(), args)));
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
        let r = match &f.body.tail {
            Some(t) => self.eval(t, &mut env),
            None => Err(Halt::Stuck),
        };
        self.depth -= 1;
        match r {
            Ok(v) => Ok(v),
            Err(Halt::Stuck) => Ok(neu(Term::Call(name.into(), args))),
            Err(Halt::Err(e)) => Err(e),
        }
    }

    pub(super) fn eval(&mut self, e: &Expr, env: &mut Env) -> H<V> {
        let mut e = e;
        while let Expr::Id(_, inner) | Expr::InModule(_, inner) = e { e = inner; }
        Ok(match e {
            Expr::At(_, x) | Expr::Id(_, x) | Expr::InModule(_, x) => self.eval(x, env)?,
            Expr::Int(n) => V::Int(*n),
            Expr::Num(n) => V::Num(*n),
            Expr::Bool(b) => V::Bool(*b),
            Expr::Str(s) => V::Str(s.as_str().into()),
            Expr::Var(n) => match env.iter().rev().find_map(|s| s.get(n)) {
                Some(v) => v.clone(),
                None if n == "Type" => V::Type,
                None if PRIMS.contains(&n.as_str()) => prim(n),
                None if self
                    .fns
                    .get(n)
                    .is_some_and(|f| f.alias && f.params.is_empty()) =>
                {
                    self.call(n, vec![]).map_err(Halt::Err)?
                }
                None if self.ctors.contains_key(n)
                    || self.datas.contains_key(n) =>
                {
                    self.call(n, vec![]).map_err(Halt::Err)?
                }
                None if self.fns.contains_key(n) => return Err(Halt::Stuck),
                None => return Err(Halt::Err(format!("未定義の変数 {n}").into())),
            },
            Expr::Unary(op, x) => un(op, self.eval(x, env)?),
            Expr::Binary(op, l, r) => {
                let a = self.eval(l, env)?;
                // 左辺で決まる && と || は右辺を評価しない
                match (*op, &a) {
                    ("&&", V::Bool(false)) | ("||", V::Bool(true)) => return Ok(a),
                    _ => {}
                }
                let b = self.eval(r, env)?;
                bin(op, a, b)
            }
            Expr::Call(name, args) if name == "#num" => num(self.eval(&args[0], env)?),
            Expr::Call(name, _) if name == "put" || SYN_FNS.iter().any(|s| s.0 == name) => {
                return Err(Halt::Stuck);
            }
            Expr::Call(name, args) => {
                let mut vs = Vec::new();
                for a in args {
                    vs.push(self.eval(a, env)?);
                }
                match env.iter().rev().find_map(|s| s.get(name)).cloned() {
                    Some(_) => return Err(Halt::Stuck),
                    None => self.call(name, vs).map_err(Halt::Err)?,
                }
            }
            Expr::Pi(binders, ret, effs) => self.eval_pi(binders, ret, effs, env)?,
            Expr::Array(items) => {
                let mut vs = Vec::new();
                for a in items {
                    vs.push(self.eval(a, env)?);
                }
                V::Arr(Rc::new(vs))
            }
            Expr::Field(x, f) => {
                let v = self.eval(x, env)?;
                if matches!(v, V::Prim("syn") | V::Syn(_)) {
                    return Err(Halt::Stuck);
                }
                if !matches!(v, V::Data(_) | V::Neu(_)) {
                    return Err(Halt::Stuck);
                }
                self.proj(v, f)
            }
            Expr::Index(v, i) => match (self.eval(v, env)?, self.eval(i, env)?) {
                (V::Arr(xs), V::Int(i)) if usize::try_from(i).is_ok_and(|i| i < xs.len()) => {
                    xs[i as usize].clone()
                }
                // 値の分からない配列や添字は v[i] という項にする（範囲は型検査で証明済み）
                (v @ V::Neu(_), i) | (v, i @ V::Neu(_)) => neu(Term::Call("[]".into(), vec![v, i])),
                _ => return Err(Halt::Stuck),
            },
            Expr::Refine(b, x, p) => {
                let b = self.eval(b, env)?;
                env.push(HashMap::from([(x.clone(), bound())]));
                let p = self.eval_or_opaque(p, env);
                env.pop();
                V::Refine(Rc::new((b, p.map_err(Halt::Err)?)))
            }
            Expr::Apply(..)
            | Expr::Lam(..)
            | Expr::Match(..)
            | Expr::If(..)
            | Expr::Block(..)
            | Expr::Assign(..)
            | Expr::For(..)
            | Expr::While(..)
            | Expr::Break
            | Expr::Continue
            | Expr::Return(_)
            | Expr::Handle(..)
            | Expr::Exit(..)
            | Expr::Method(..)
            | Expr::Await(_)
            | Expr::Quote(_)
            | Expr::Macro(..)
            | Expr::Try(_) => return Err(Halt::Stuck),
        })
    }

    // 変数 id を by に置き換え、止まっていた計算を進め直す
    pub(super) fn subst(&mut self, v: &V, id: usize, by: &V) -> R<V> {
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
                effs: p
                    .effs
                    .iter()
                    .map(|x| self.subst(x, id, by))
                    .collect::<R<_>>()?,
            })),
            V::Neu(n) => match &**n {
                Term::Var(i, _) if *i == id => by.clone(),
                Term::Var(..) | Term::Opaque(_) => v.clone(),
                Term::Num(x) => num(self.subst(x, id, by)?),
                Term::Un(op, x) => un(op, self.subst(x, id, by)?),
                Term::Bin(op, a, b) => {
                    let (a, b) = (self.subst(a, id, by)?, self.subst(b, id, by)?);
                    bin(op, a, b)
                }
                Term::Call(f, args) => {
                    let args = args
                        .iter()
                        .map(|a| self.subst(a, id, by))
                        .collect::<R<_>>()?;
                    self.call(f, args)?
                }
                Term::Proj(x, f) => {
                    let x = self.subst(x, id, by)?;
                    self.proj(x, f)
                }
            },
            v => v.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_calls_get_distinct_opaque_terms() {
        let prog = crate::parser::parse(
            "fn runtime() -> int { 1 }\nfn delayed() -> int with Async { 1 }",
        ).unwrap();
        let mut eval = Eval::new();
        eval.fns = prog.fns.into_iter().map(|f| (f.name.clone(), f)).collect();
        for name in [
            "read", "input", "now_ms", "new_ref", "write", "put", "panic", "all",
            "spawn", "task_then", "task_done", "task_value", "task_wait", "runtime",
            "delayed", "unknown",
        ] {
            let args = vec![bound()];
            let a = eval.call(&name.into(), args.clone()).unwrap();
            let b = eval.call(&name.into(), args).unwrap();
            assert!(matches!(&a, V::Neu(n) if matches!(&**n, Term::Opaque(_))), "{name}: {a:?}");
            assert_ne!(a, b, "{name}");
        }
    }

    #[test]
    fn pure_symbolic_calls_survive_substitution() {
        let mut eval = Eval::new();
        let pushed = eval.call(&"push".into(), vec![bound(), V::Int(2)]).unwrap();
        let length = eval.call(&"len".into(), vec![pushed]).unwrap();
        let xs = V::Arr(Rc::new(vec![V::Int(1)]));
        assert_eq!(eval.subst(&length, 0, &xs).unwrap(), V::Int(2));

        let indexed = eval.call(&"[]".into(), vec![bound(), V::Int(0)]).unwrap();
        assert_eq!(eval.subst(&indexed, 0, &xs).unwrap(),
            neu(Term::Call("[]".into(), vec![xs, V::Int(0)])));
    }
}
