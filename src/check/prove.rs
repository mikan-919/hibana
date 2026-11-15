//! 細分化型の項と事実の照合。
use super::typeck::{neu, Neu, V};

// 項と定数の和に分ける。定数だけなら項は 0
fn lin(v: &V) -> (V, i64) {
    match v {
        V::Int(c) => (V::Int(0), *c),
        V::Neu(m) => match &**m {
            Neu::Bin("+", x, V::Int(c)) => (x.clone(), *c),
            Neu::Bin("-", x, V::Int(c)) => (x.clone(), c.saturating_neg()),
            _ => (v.clone(), 0),
        },
        _ => (v.clone(), 0),
    }
}

// 比較を a <= b + k の形に直す。a < b は a <= b - 1。両辺の定数は k に寄せる（2 <= n は 0 <= n - 2）
fn le_form(v: &V) -> Option<(V, V, i64)> {
    let V::Neu(n) = v else { return None };
    let (a, b, k): (&V, &V, i64) = match &**n {
        Neu::Bin("<=", a, b) => (a, b, 0),
        Neu::Bin("<", a, b) => (a, b, -1),
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
        Neu::Bin(op, a, b) => bin(op, replace(a, from, to), replace(b, from, to)),
        Neu::Un(op, x) => un(op, replace(x, from, to)),
        _ => v.clone(),
    }
}

// 項の中の len(変数 id) を t に置き換える（足し算・引き算の中だけ）
pub(super) fn replace_len(v: &V, id: usize, t: &V) -> V {
    if let V::Neu(n) = v {
        match &**n {
            Neu::Call(f, a)
                if f == "len"
                    && matches!(&a[..], [V::Neu(x)] if matches!(&**x, Neu::Var(i, _) if *i == id)) =>
            {
                return t.clone();
            }
            Neu::Bin(op, a, b) if matches!(*op, "+" | "-") => {
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
            Neu::Bin("&&", a, b) => return prove_facts(a, facts.clone()) && prove_facts(b, facts.clone()),
            Neu::Bin("||", a, b) => return prove_facts(a, facts.clone()) || prove_facts(b, facts.clone()),
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
            (Neu::Bin("<", a, b), Neu::Bin("!=", c, d)) => {
                (a == c && b == d) || (a == d && b == c)
            }
            _ => false,
        }
    };
    *goal == V::Bool(true) || facts.clone().any(implies)
}

// 事実との照合と定数の代入で示し、残りを Z3 に問い合わせる。
pub(super) fn prove<'a>(goal: &V, facts: impl Iterator<Item = &'a V> + Clone, smt: &mut super::smt::Smt) -> bool {
    if prove_facts(goal, facts.clone()) { return true; }
    let eqs: Vec<_> = facts.clone().filter_map(|f| match f {
        V::Neu(n) => match &**n {
            Neu::Bin("==", x, c @ V::Int(_)) | Neu::Bin("==", c @ V::Int(_), x) => Some((x, c)),
            _ => None,
        },
        _ => None,
    }).collect();
    if !eqs.is_empty() {
        let g = eqs.iter().fold(goal.clone(), |g, (x, c)| replace(&g, x, c));
        if g == V::Bool(true) || prove_facts(&g, facts.clone()) { return true; }
    }
    smt.prove(&facts.cloned().collect::<Vec<_>>(), goal)
}

pub(super) fn un(op: &'static str, v: V) -> V {
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
            return bin(o, a.clone(), b.clone());
        }
    }
    match (op, v) {
        ("-", V::Int(n)) if n != i64::MIN => V::Int(-n),
        ("!", V::Bool(b)) => V::Bool(!b),
        (op, v) => neu(Neu::Un(op, v)),
    }
}

pub(super) fn bin(op: &'static str, a: V, b: V) -> V {
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
        return r;
    }
    // > と >= は向きを揃えて < と <= にする
    match op {
        ">" => neu(super::typeck::Neu::Bin("<", b, a)),
        ">=" => neu(super::typeck::Neu::Bin("<=", b, a)),
        _ => neu(super::typeck::Neu::Bin(op, a, b)),
    }
}

pub(super) fn len(v: V) -> V {
    match &v {
        V::Arr(xs) => V::Int(xs.len() as i64),
        V::Str(s) => V::Int(s.chars().count() as i64),
        V::Neu(n) => match &**n {
            // push は 1 つ増やし、set_at は変えない
            Neu::Call(f, a) if f == "push" && a.len() == 2 => {
                bin("+", len(a[0].clone()), V::Int(1))
            }
            Neu::Call(f, a) if f == "set_at" && a.len() == 3 => len(a[0].clone()),
            _ => neu(Neu::Call("len".into(), vec![v.clone()])),
        },
        _ => neu(Neu::Call("len".into(), vec![v])),
    }
}
