// 型検査の証明器で示せなかった条件を、Z3 に問い合わせる（z3 クレートで Z3 のライブラリを直接呼ぶ）
use crate::typeck::{Neu, V};
use std::collections::HashMap;
use z3::ast::{Bool, Int};
use z3::{SatResult, Solver};

// ponytail: int を上限の無い整数として扱う。オーバーフローは実行時に検査している。厳密にするなら BitVec 64 にする
pub struct Smt {
    solver: Option<Solver>,
}

// Z3 の項。int か bool
enum T {
    I(Int),
    B(Bool),
}

impl Smt {
    pub fn new() -> Self {
        Smt { solver: None }
    }

    // facts がすべて真なら goal も真か。答えが出ない時は false
    pub fn prove(&mut self, facts: &[V], goal: &V) -> bool {
        let s = self.solver.get_or_insert_with(Solver::new);
        s.push();
        let mut tr = Tr {
            atoms: HashMap::new(),
        };
        for f in facts {
            s.assert(tr.bool(f));
        }
        // goal の否定が充足不能なら証明できたことになる
        s.assert(tr.bool(goal).not());
        let r = s.check() == SatResult::Unsat;
        s.pop(1);
        r
    }
}

// V を Z3 の項に訳す。訳せない項（関数呼び出し・変数など）は、同じ項ごとに1個の定数にする
struct Tr {
    atoms: HashMap<(String, bool), T>,
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

impl Tr {
    fn int(&mut self, v: &V) -> Int {
        match self.term(v, true) {
            T::I(i) => i,
            T::B(_) => unreachable!("int として訳した"),
        }
    }

    fn bool(&mut self, v: &V) -> Bool {
        match self.term(v, false) {
            T::B(b) => b,
            T::I(_) => unreachable!("bool として訳した"),
        }
    }

    // int なら Int、そうでなければ Bool の項にする
    fn term(&mut self, v: &V, int: bool) -> T {
        match (v, int) {
            (V::Int(n), true) => return T::I(Int::from_i64(*n)),
            (V::Bool(b), false) => return T::B(Bool::from_bool(*b)),
            (V::Neu(n), _) => {
                let r = match (&**n, int) {
                    (Neu::Un("-", x), true) => Some(T::I(-self.int(x))),
                    (Neu::Un("!", x), false) => Some(T::B(self.bool(x).not())),
                    (Neu::Bin(op @ ("+" | "-" | "*"), a, b), true) => {
                        let xs = [self.int(a), self.int(b)];
                        Some(T::I(match *op {
                            "+" => Int::add(&xs),
                            "-" => Int::sub(&xs),
                            _ => Int::mul(&xs),
                        }))
                    }
                    (Neu::Bin(op @ ("<" | "<="), a, b), false) => {
                        let (a, b) = (self.int(a), self.int(b));
                        Some(T::B(if *op == "<" { a.lt(&b) } else { a.le(&b) }))
                    }
                    (Neu::Bin(op @ ("&&" | "||"), a, b), false) => {
                        let xs = [self.bool(a), self.bool(b)];
                        Some(T::B(if *op == "&&" {
                            Bool::and(&xs)
                        } else {
                            Bool::or(&xs)
                        }))
                    }
                    (Neu::Bin(op @ ("==" | "!="), a, b), false) => {
                        let eq = match sort_of(a).or(sort_of(b)).unwrap_or(true) {
                            true => self.int(a).eq(self.int(b)),
                            false => self.bool(a).eq(self.bool(b)),
                        };
                        Some(T::B(if *op == "==" { eq } else { eq.not() }))
                    }
                    _ => None,
                };
                if let Some(t) = r {
                    return t;
                }
            }
            _ => {}
        }
        let key = (format!("{v:?}"), int);
        if let Some(a) = self.atoms.get(&key) {
            return match a {
                T::I(i) => T::I(i.clone()),
                T::B(b) => T::B(b.clone()),
            };
        }
        let name = format!("t{}", self.atoms.len());
        let (a, b) = if int {
            let c = Int::new_const(name);
            (T::I(c.clone()), T::I(c))
        } else {
            let c = Bool::new_const(name);
            (T::B(c.clone()), T::B(c))
        };
        self.atoms.insert(key, a);
        b
    }
}
