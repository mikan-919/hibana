use crate::parser::Name;
// 名前の解決。修飾していない大域の名前を、そのモジュールから見た修飾した名前（web.ok など）に書き換える。
// 局所変数（引数・let・無名関数の引数・match の束縛・for の変数）は書き換えない
use crate::parser::{Block, Expr, Program, Stmt, Use, qual};
use std::collections::{HashMap, HashSet};
use crate::error::{Error, R};
use crate::lexer::Span;

// 構築子とトレイトから生成した名前も、元の宣言の公開範囲を持つ
fn visibility(prog: &Program) -> HashMap<Name, (String, bool)> {
    let mut names: HashMap<_, _> = prog.fns.iter()
        .map(|f| (f.name.clone(), (f.module.clone(), f.public))).collect();
    for data in &prog.datas {
        names.insert(data.name.clone(), (data.module.clone(), data.public));
        for ctor in &data.ctors {
            names.insert(ctor.name.clone(), (data.module.clone(), data.public));
        }
    }
    names
}

pub fn resolve(
    prog: &mut Program,
    uses: &HashMap<String, Vec<Use>>,
    known: &HashSet<Name>,
    grammar: &crate::meta::Grammar,
) -> R<()> {
    let visibility = visibility(prog);
    let ctors: HashMap<Name, Name> = prog.datas.iter().flat_map(|data| {
        data.ctors.iter().map(move |ctor| (Name::associated(data.name.clone(), ctor.name.local().clone()), ctor.name.clone()))
    }).collect();
    let mut imports: Vec<_> = uses.iter().collect();
    imports.sort_by_key(|(module, _)| *module);
    for (module, imports) in imports {
        for import in imports {
            if let crate::parser::UseNames::List(names) = &import.names {
                for name in names {
                    let qualified = qual(&import.module, name);
                    let resolved = ctors.get(&qualified).unwrap_or(&qualified);
                    if let Some((owner, false)) = visibility.get(resolved)
                        && owner != module
                        && !grammar.declares(&qualified)
                    {
                        let error = Error::from(format!("{qualified} は非公開（宣言に pub を付ける）"));
                        return Err(match import.span { Some(span) => error.at(span), None => error });
                    }
                }
            }
        }
    }
    let ctors = &ctors;
    for f in &mut prog.fns {
        let mut r = Resolver {
            module: f.module.clone(),
            uses,
            known,
            ctors,
            locals: vec![Vec::new()],
            visibility: &visibility,
            span: f.span,
            error: None,
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
        if let Some(error) = r.error { return Err(error); }
    }
    for d in &mut prog.datas {
        let mut r = Resolver {
            module: d.module.clone(),
            uses,
            known,
            ctors,
            locals: vec![Vec::new()],
            visibility: &visibility,
            span: d.span,
            error: None,
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
        if let Some(error) = r.error { return Err(error); }
    }
    public_signatures(prog)
}

// 公開宣言の型（引数・戻り値・with・フィールド・別名の右辺）に、同じモジュールの非公開の型を含められない。
// 含めると、他のモジュールが名前を書けない型の値を受け取れてしまう
fn public_signatures(prog: &Program) -> R<()> {
    let private: HashSet<&Name> = prog.datas.iter().filter(|d| !d.public).map(|d| &d.name)
        .chain(prog.fns.iter().filter(|f| f.alias && !f.public).map(|f| &f.name))
        .collect();
    let find = |ty: &Expr| {
        let mut found = None;
        names_in(ty, &mut |n| if found.is_none() && private.contains(n) { found = Some(n.clone()) });
        found
    };
    let fail = |decl: &Name, ty: Name, span: Option<Span>| {
        let error = Error::from(format!("公開宣言 {decl} に非公開型 {ty} を含められない"));
        Err(match span { Some(span) => error.at(span), None => error })
    };
    for f in prog.fns.iter().filter(|f| f.public) {
        let alias = f.alias.then_some(f.body.tail.as_deref()).flatten();
        let types = f.params.iter().map(|p| &p.1).chain(&f.ret).chain(&f.effects).chain(alias);
        for ty in types {
            if let Some(n) = find(ty) { return fail(&f.name, n, f.span); }
        }
    }
    for d in prog.datas.iter().filter(|d| d.public) {
        let types = d.params.iter().map(|p| &p.1).chain(d.ctors.iter().flat_map(|c| c.fields.iter().map(|f| &f.1)));
        for ty in types {
            if let Some(n) = find(ty) { return fail(&d.name, n, d.span); }
        }
    }
    Ok(())
}

// 型の式に現れる大域の名前。細分化型の条件は見ない
fn names_in(e: &Expr, f: &mut impl FnMut(&Name)) {
    match e.peel() {
        Expr::Var(n) => f(n),
        Expr::Call(n, xs) => {
            f(n);
            xs.iter().for_each(|x| names_in(x, f));
        }
        Expr::Refine(b, _, _) => names_in(b, f),
        Expr::Pi(ps, r, effs) => {
            ps.iter().for_each(|p| names_in(&p.1, f));
            names_in(r, f);
            effs.iter().for_each(|x| names_in(x, f));
        }
        Expr::Array(xs) => xs.iter().for_each(|x| names_in(x, f)),
        Expr::Apply(g, xs) => {
            names_in(g, f);
            xs.iter().for_each(|x| names_in(x, f));
        }
        Expr::Block(b) => if let Some(t) = &b.tail { names_in(t, f) },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_values_shadow_module_names() {
        let mut modules = crate::parser::ModuleSet::from("use tools\nstruct Data { x: int }\nfn main() { put(tools.x())\n let tools = Data { x: 3 }\n put(tools.x) }");
        let mut tools = crate::parser::ModuleSet::from("pub fn x() -> int with {} { 7 }").modules.remove(0);
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
    prog: &Program,
) -> R<()> {
    let visibility = visibility(prog);
    let mut r = Resolver {
        module: module.into(),
        uses,
        known,
        ctors: &HashMap::new(),
        locals: vec![params],
        visibility: &visibility,
        span: None,
        error: None,
    };
    r.block(block);
    match r.error { Some(error) => Err(error), None => Ok(()) }
}

struct Resolver<'a> {
    module: String,
    uses: &'a HashMap<String, Vec<Use>>,
    known: &'a HashSet<Name>,
    ctors: &'a HashMap<Name, Name>,
    locals: Vec<Vec<Name>>,
    visibility: &'a HashMap<Name, (String, bool)>,
    span: Option<Span>,
    error: Option<Error>,
}

impl Resolver<'_> {
    // 自分のモジュール、use で取り込んだ名前の順に探す。利用者のファイル（module が ""）では、自分の名前は修飾しない形のまま
    fn name(&mut self, name: &mut Name) {
        if self.locals.iter().any(|scope| scope.contains(name)) { return; }
        let mut candidates = vec![qual(&self.module, &*name)];
        for import in self.uses.get(&self.module).into_iter().flatten() {
            if import.names.takes(name) { candidates.push(qual(&import.module, &*name)); }
        }
        candidates.push(name.clone());
        let mut private = None;
        for candidate in candidates {
            let resolved = self.ctors.get(&candidate).unwrap_or(&candidate);
            if self.known.contains(resolved) || self.visibility.contains_key(resolved) {
                if let Some((owner, false)) = self.visibility.get(resolved)
                    && *owner != self.module
                {
                    private = Some(resolved.clone());
                    continue;
                }
                *name = resolved.clone();
                return;
            }
        }
        if let Some(private) = private { self.private(private); }
    }

    fn private(&mut self, name: Name) {
        if self.error.is_none() {
            let error = Error::from(format!("{name} は非公開（宣言に pub を付ける）"));
            self.error = Some(match self.span { Some(span) => error.at(span), None => error });
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
            Expr::At(span, x) => {
                let previous = self.span.replace(*span);
                self.expr(x);
                self.span = previous;
            }
            Expr::InModule(module, x) => {
                let outer = std::mem::replace(&mut self.module, module.clone());
                self.expr(x);
                self.module = outer;
            }
            Expr::Try(x) => self.expr(x),
            Expr::Id(_, x)
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
                    // 腕の操作名は構文解析で修飾済み。他のモジュールの非公開トレイトは提供できない
                    if let Some((owner, false)) = self.visibility.get(&a.ctor)
                        && *owner != self.module
                    {
                        self.private(a.ctor.clone());
                    }
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
                if let Some(effs) = effs { self.exprs(effs); }
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
            | Expr::Break
            | Expr::Continue
            | Expr::Macro(..)
            | Expr::Quote(_) => {}
        }
    }
}
