// 名前の解決。修飾していない大域の名前を、そのモジュールから見た修飾した名前（web.ok など）に書き換える。
// 局所変数（引数・let・無名関数の引数・match の束縛・for の変数）は書き換えない
use crate::parser::{Block, Expr, Program, Stmt, Use, qual};
use std::collections::{HashMap, HashSet};

pub fn resolve(prog: &mut Program, uses: &HashMap<String, Vec<Use>>, known: &HashSet<String>) {
    // 型の名前::構築子（Shape::Circle は字句解析で Shape__Circle になっている）から構築子の名前
    // ponytail: 型の名前は修飾を外して引く。別のモジュールに同じ名前の型と構築子があれば後の方になる
    let last = |n: &str| n.rsplit('.').next().unwrap_or(n).to_string();
    let ctors: HashMap<String, String> = prog
        .datas
        .iter()
        .flat_map(|d| d.ctors.iter().map(move |c| (format!("{}__{}", last(&d.name), last(&c.name)), last(&c.name))))
        .collect();
    let ctors = &ctors;
    for f in &mut prog.fns {
        let mut r = Resolver {
            module: f.module.clone(),
            uses,
            known,
            ctors,
            locals: vec![Vec::new()],
        };
        for (n, t) in &mut f.params {
            r.expr(t);
            r.locals.last_mut().unwrap().push(n.clone());
        }
        if let Some(t) = &mut f.ret {
            r.expr(t);
        }
        for t in &mut f.effects {
            r.expr(t);
        }
        r.block(&mut f.body);
    }
    for d in &mut prog.datas {
        let mut r = Resolver {
            module: d.module.clone(),
            uses,
            known,
            ctors,
            locals: vec![Vec::new()],
        };
        for (n, t) in &mut d.params {
            r.expr(t);
            r.locals.last_mut().unwrap().push(n.clone());
        }
        for c in &mut d.ctors {
            r.locals.push(Vec::new());
            for (n, t) in &mut c.fields {
                r.expr(t);
                r.locals.last_mut().unwrap().push(n.clone());
            }
            r.locals.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_values_shadow_module_names() {
        let mut program = crate::parser::parse("use tools\nstruct Data { x: int }\nfn main() { put(tools.x())\n let tools = Data { x: 3 }\n put(tools.x) }\nmodule tools\nfn x() -> int { 7 }").unwrap();
        crate::typeck::check(&mut program).unwrap();
        let mut output = Vec::new();
        crate::eval::run(&program, &mut output).unwrap();
        assert_eq!(output, b"7\n3\n");
    }
}

pub(crate) fn resolve_macro(
    block: &mut Block,
    module: &str,
    params: Vec<String>,
    uses: &HashMap<String, Vec<Use>>,
    known: &HashSet<String>,
) {
    let mut r = Resolver {
        module: module.into(),
        uses,
        known,
        ctors: &HashMap::new(),
        locals: vec![params],
    };
    r.block(block);
}

struct Resolver<'a> {
    module: String,
    uses: &'a HashMap<String, Vec<Use>>,
    known: &'a HashSet<String>,
    ctors: &'a HashMap<String, String>,
    locals: Vec<Vec<String>>,
}

impl Resolver<'_> {
    // 自分のモジュール、use で取り込んだ名前の順に探す。利用者のファイル（module が ""）では、自分の名前は修飾しない形のまま
    fn name(&self, n: &mut String) {
        if let Some(c) = self.ctors.get(n.as_str())
            && !self.locals.iter().any(|s| s.contains(n))
        {
            *n = c.clone();
        }
        if n.contains('.') || self.locals.iter().any(|s| s.contains(n)) {
            return;
        }
        let mut cands = vec![qual(&self.module, n)];
        for u in self.uses.get(&self.module).into_iter().flatten() {
            let take = u.names.takes(n);
            if take {
                cands.push(qual(&u.module, n));
            }
        }
        if let Some(c) = cands.into_iter().find(|c| self.known.contains(c)) {
            *n = c;
        }
    }

    fn with<T>(&mut self, names: &[String], f: impl FnOnce(&mut Self) -> T) -> T {
        self.locals.push(names.to_vec());
        let r = f(self);
        self.locals.pop();
        r
    }

    fn block(&mut self, b: &mut Block) {
        self.locals.push(Vec::new());
        for s in &mut b.stmts {
            match s {
                Stmt::Let { name, ty, init, .. } => {
                    if let Some(t) = ty {
                        self.expr(t);
                    }
                    self.expr(init);
                    self.locals.last_mut().unwrap().push(name.clone());
                }
                Stmt::Expr(e) => self.expr(e),
            }
        }
        if let Some(t) = &mut b.tail {
            self.expr(t);
        }
        self.locals.pop();
    }

    fn exprs(&mut self, es: &mut [Expr]) {
        for e in es {
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        let local_member = match &*e {
            Expr::Var(n) | Expr::Call(n, _) => n.split_once('.').filter(|(base, _)| self.locals.iter().any(|s| s.iter().any(|n| n == base))).map(|(base, field)| (base.to_string(), field.to_string())),
            _ => None,
        };
        if let Some((base, field)) = local_member {
            let old = std::mem::replace(e, Expr::Int(0));
            *e = match old {
                Expr::Call(_, args) => Expr::Method(Box::new(Expr::Var(base)), field, args),
                _ => Expr::Field(Box::new(Expr::Var(base)), field),
            };
            self.expr(e);
            return;
        }
        match e {
            Expr::Var(n) => self.name(n),
            Expr::Call(n, args) => {
                self.name(n);
                self.exprs(args);
            }
            Expr::At(_, x)
            | Expr::Unary(_, x)
            | Expr::Field(x, _)
            | Expr::Assign(_, x)
            | Expr::Return(Some(x)) => self.expr(x),
            Expr::Binary(_, a, b) | Expr::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::Refine(b, x, p) => {
                self.expr(b);
                let x = x.clone();
                self.with(&[x], |r| r.expr(p));
            }
            Expr::Array(xs) => self.exprs(xs),
            Expr::Apply(f, args) | Expr::Method(f, _, args) => {
                self.expr(f);
                self.exprs(args);
            }
            Expr::If(c, t, e) => {
                self.expr(c);
                self.block(t);
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            Expr::While(c, b) => {
                self.expr(c);
                self.block(b);
            }
            Expr::For(x, a, b, body) => {
                self.expr(a);
                if let Some(b) = b {
                    self.expr(b);
                }
                let x = x.clone();
                self.with(&[x], |r| r.block(body));
            }
            Expr::Block(b) => self.block(b),
            Expr::Match(s, arms) | Expr::Handle(s, arms) => {
                self.expr(s);
                for a in arms {
                    if a.ctor != "_" {
                        self.name(&mut a.ctor);
                    }
                    let binds = a.binds.clone();
                    self.with(&binds, |r| r.expr(&mut a.body));
                }
            }
            Expr::Lam(params, ret, effs, body) => {
                self.locals.push(Vec::new());
                for (n, t) in params.iter_mut() {
                    if let Some(t) = t {
                        self.expr(t);
                    }
                    self.locals.last_mut().unwrap().push(n.clone());
                }
                if let Some(t) = ret {
                    self.expr(t);
                }
                self.exprs(effs);
                self.block(body);
                self.locals.pop();
            }
            Expr::Pi(binders, ret, effs) => {
                self.locals.push(Vec::new());
                for (n, t) in binders.iter_mut() {
                    self.expr(t);
                    self.locals.last_mut().unwrap().push(n.clone());
                }
                self.expr(ret);
                self.exprs(effs);
                self.locals.pop();
            }
            Expr::Int(_)
            | Expr::Num(_)
            | Expr::Bool(_)
            | Expr::Str(_)
            | Expr::Return(None)
            | Expr::Macro(..)
            | Expr::Quote(_) => {}
        }
    }
}
