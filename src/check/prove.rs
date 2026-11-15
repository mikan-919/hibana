//! 細分化型の項と事実の照合。
use super::typeck::{Neu, Norm, V};

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
        Neu::Bin(op, a, b) => Norm::bin(op, replace(a, from, to), replace(b, from, to)),
        Neu::Un(op, x) => Norm::un(op, replace(x, from, to)),
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
                return Norm::bin(op, replace_len(a, id, t), replace_len(b, id, t));
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

