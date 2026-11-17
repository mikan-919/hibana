use crate::parser::Name;
// 名前の解決。修飾していない大域の名前を、そのモジュールから見た修飾した名前（web.ok など）に書き換える。
// 局所変数（引数・let・無名関数の引数・match の束縛・for の変数）は書き換えない
use crate::parser::{Block, Expr, Program, Stmt, Use, qual};
use std::collections::{HashMap, HashSet};

pub fn resolve(prog: &mut Program, uses: &HashMap<String, Vec<Use>>, known: &HashSet<Name>) {
    let ctors: HashMap<Name, Name> = prog.datas.iter().flat_map(|data| {
        data.ctors.iter().map(move |ctor| (Name::associated(data.name.clone(), ctor.name.local().clone()), ctor.name.clone()))
    }).collect();
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
        let mut modules = crate::parser::ModuleSet::from("use tools\nstruct Data { x: int }\nfn main() { put(tools.x())\n let tools = Data { x: 3 }\n put(tools.x) }");
        let mut tools = crate::parser::ModuleSet::from("fn x() -> int { 7 }").modules.remove(0);
        tools.name = "tools".into();
        tools.source.id = crate::frontend::source::SourceId(1);
        modules.modules.push(tools);
        let mut program = crate::parser::parse_modules(modules).unwrap();
        crate::typeck::check(&mut program).unwrap();
        let mut output = Vec::new();
        crate::eval::run(&program, &mut output).unwrap();
        assert_eq!(output, b"7\n3\n");
    }
}

pub(crate) fn resolve_macro(
    block: &mut Block,
    module: &str,
    params: Vec<Name>,
    uses: &HashMap<String, Vec<Use>>,
    known: &HashSet<Name>,
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
    known: &'a HashSet<Name>,
    ctors: &'a HashMap<Name, Name>,
    locals: Vec<Vec<Name>>,
}

impl Resolver<'_> {
    // 自分のモジュール、use で取り込んだ名前の順に探す。利用者のファイル（module が ""）では、自分の名前は修飾しない形のまま
    fn name(&self, name: &mut Name) {
        if self.locals.iter().any(|scope| scope.contains(name)) { return; }
        let mut candidates = vec![qual(&self.module, &*name)];
        for import in self.uses.get(&self.module).into_iter().flatten() {
            if import.names.takes(name) { candidates.push(qual(&import.module, &*name)); }
        }
        candidates.push(name.clone());
        for candidate in candidates {
            let resolved = self.ctors.get(&candidate).unwrap_or(&candidate);
            if self.known.contains(resolved) { *name = resolved.clone(); return; }
        }
    }

    fn with<T>(&mut self, names: &[Name], f: impl FnOnce(&mut Self) -> T) -> T {
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
            Expr::Var(n) | Expr::Call(n, _) => match &n.kind {
                crate::parser::NameKind::Qualified(base, field) if self.locals.iter().any(|scope| scope.iter().any(|name| name == base)) => Some((Name::from(base), *field.clone())),
                _ => None,
            },
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
            Expr::At(_, x) | Expr::Id(_, x)
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
            Expr::Await(x) => self.expr(x),
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
            Expr::Handle(_, s, arms) => {
                self.expr(s);
                for a in arms {
                    let binds = a.binds.clone();
                    self.with(&binds, |r| r.expr(&mut a.body));
                }
            }
            Expr::Exit(_, x) => self.expr(x),
            Expr::Match(s, arms) => {
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
