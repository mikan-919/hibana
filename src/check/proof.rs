//! 型検査と証明用の評価器の接続。
use super::*;


impl Ck {
    pub(super) fn push(&mut self) {
        self.scopes.push(Scope::default());
    }

    pub(super) fn pop(&mut self) {
        self.scopes.pop();
    }

    // 構造体の値なら、フィールドの型の細分化の条件を v.f についての事実として加える
    pub(super) fn struct_facts(&mut self, ty: &V, val: &V) -> R<()> {
        let V::DataT(d) = base(ty) else { return Ok(()) };
        // Ref と Async は宣言の無い組み込みの型
        let Some(data) = self.norm.datas.get(&d.0).cloned() else {
            return Ok(());
        };
        let [c] = data.ctors.as_slice() else {
            return Ok(());
        };
        if c.name.structure_owner().is_none() {
            return Ok(());
        }
        let mut env: Env = vec![
            data.params
                .iter()
                .map(|p| p.0.clone())
                .zip(d.1.iter().cloned())
                .collect(),
        ];
        for (n, ft) in &c.fields {
            let t = self.norm.eval_or_opaque(ft, &mut env)?;
            let fv = self.norm.proj(val.clone(), n);
            for p in split(&t).1 {
                let f = self.norm.subst(&p, 0, &fv)?;
                self.add_fact(f);
            }
            env[0].insert(n.clone(), fv);
        }
        Ok(())
    }

    // 変数を束縛し、型の細分化の条件をその変数についての事実として加える
    pub(super) fn bind(&mut self, name: &Name, ty: V, val: V, mutable: bool) -> R<()> {
        let val = if base(&ty) == prim("number") { crate::check::prove::num(val) } else { val };
        for p in split(&ty).1 {
            let f = self.norm.subst(&p, 0, &val)?;
            self.add_fact(f);
        }
        self.struct_facts(&ty, &val)?;
        self.scopes
            .last_mut()
            .unwrap()
            .vars
            .insert(name.clone(), Entry { ty, val, mutable });
        Ok(())
    }

    pub(super) fn lookup(&self, name: &Name) -> Option<&Entry> {
        self.scopes.iter().rev().find_map(|s| s.vars.get(name))
    }

    pub(super) fn add_fact(&mut self, v: V) {
        if let V::Neu(n) = &v
            && let Term::Bin("&&", a, b) = &**n
        {
            self.add_fact(a.clone());
            self.add_fact(b.clone());
        } else if v != V::Bool(true) {
            self.scopes.last_mut().unwrap().facts.push(v);
        }
    }

    // name = x の後の len(name) の値。x の長さが len(name) を含むなら、len(name) == t の事実で置き換える
    pub(super) fn new_len(&mut self, name: &Name, x: &Expr) -> R<Option<(V, V)>> {
        let val = self.lookup(name).unwrap().val.clone();
        let V::Neu(n) = &val else { return Ok(None) };
        let Term::Var(id, _) = &**n else {
            return Ok(None);
        };
        let id = *id;
        let mut l = len(self.eval_value(x)?);
        if mentions(&l, id) {
            let own = len(val.clone());
            let known = self
                .scopes
                .iter()
                .flat_map(|s| &s.facts)
                .find_map(|f| match f {
                    V::Neu(n) => match &**n {
                        Term::Bin("==", a, b) if *a == own && !mentions(b, id) => Some(b.clone()),
                        Term::Bin("==", a, b) if *b == own && !mentions(a, id) => Some(a.clone()),
                        _ => None,
                    },
                    _ => None,
                });
            let Some(t) = known else { return Ok(None) };
            l = replace_len(&l, id, &t);
            if mentions(&l, id) {
                return Ok(None);
            }
        }
        Ok(Some((val, l)))
    }

    // 変数の値が変わるので、その変数についての事実を捨てる。型の条件は代入でも保たれるので残す
    pub(super) fn forget(&mut self, name: &Name) -> R<()> {
        let Some(en) = self.lookup(name) else {
            return Ok(());
        };
        let (ty, val) = (en.ty.clone(), en.val.clone());
        let V::Neu(n) = &val else { return Ok(()) };
        let term = if let Term::Num(V::Neu(inner)) = &**n { &**inner } else { &**n };
        let Term::Var(id, _) = term else { return Ok(()) };
        for s in &mut self.scopes {
            s.facts.retain(|f| !mentions(f, *id));
        }
        for p in split(&ty).1 {
            let f = self.norm.subst(&p, 0, &val)?;
            self.add_fact(f);
        }
        Ok(())
    }

    pub(super) fn prove(&mut self, goal: &V) -> bool {
        crate::check::prove::prove(goal, self.scopes.iter().flat_map(|s| &s.facts), &mut self.smt)
    }

    // got 型の式 e を expected 型として使えるか。expected の細分化の条件は証明する
    pub(super) fn subsume(&mut self, e: &mut Expr, got: &V, expected: &V) -> R<()> {
        let (eb, eps) = split(expected);
        let (gb, gps) = split(got);
        if eb == prim("number") && (gb == prim("int") || gb == prim("number"))
            && !matches!(e.peel(), Expr::Call(n, _) if n == "#num")
            && !matches!(e.peel(), Expr::Num(_))
        {
            let old = std::mem::replace(e, Expr::Int(0));
            *e = Expr::Call("#num".into(), vec![old]);
            if gb == prim("int") { return self.subsume(e, &prim("number"), expected); }
        }
        if !same(&eb, &gb) {
            return Err(mismatch(expected, got).into());
        }
        if eps.is_empty() || gb == prim("never") {
            return Ok(());
        }
        let v = self.eval_value(e)?;
        self.push();
        let mut r = Ok(());
        for gp in &gps {
            let f = self.norm.subst(gp, 0, &v)?;
            self.add_fact(f);
        }
        for ep in eps.iter().filter(|p| !gps.contains(p)) {
            let goal = self.norm.subst(ep, 0, &v)?;
            if !self.prove(&goal) {
                r = Err(format!("条件 {} を証明できない", show(&goal)).into());
                break;
            }
        }
        self.pop();
        r
    }

    // 評価器に渡す、変数の値の環境
    // ponytail: 評価のたびに全スコープを複製する。遅ければ永続的なマップにする
    pub(super) fn val_env(&self) -> Env {
        vec![
            self.scopes
                .iter()
                .flat_map(|s| s.vars.iter().map(|(k, e)| (k.clone(), e.val.clone())))
                .collect(),
        ]
    }

    // 0 <= i < len(v) を証明する
    pub(super) fn in_range(&mut self, v: &Expr, i: &Expr) -> R<()> {
        let (vv, iv) = (self.eval_value(v)?, self.eval_value(i)?);
        let len = len(vv);
        let goal = bin(
            "&&",
            bin("<=", V::Int(0), iv.clone()),
            bin("<", iv, len),
        );
        if !self.prove(&goal) {
            return Err(format!("添字の範囲 {} を証明できない", show(&goal)).into());
        }
        Ok(())
    }

    // 割る数 d について d != 0 を証明する
    pub(super) fn nonzero(&mut self, d: &Expr, real: bool) -> R<()> {
        let zero = if real { V::Num(0.0) } else { V::Int(0) };
        let dv = self.eval_value(d)?;
        let goal = bin("!=", dv.clone(), zero.clone());
        // Z3 を使わない証明器でも i >= 2 などの下限から示せるよう、符号の形も試す
        let signed = bin("||", bin("<", zero.clone(), dv.clone()), bin("<", dv, zero));
        if !self.prove(&goal) && !self.prove(&signed) {
            return Err(format!("割る数が 0 でないこと {} を証明できない", show(&goal)).into());
        }
        Ok(())
    }

    pub(super) fn eval_value(&mut self, e: &Expr) -> R<V> {
        let mut env = self.val_env();
        self.norm.eval_or_opaque(e, &mut env)
    }

    // 式が型であることを確かめてから、その型を計算する
    pub(super) fn eval_type(&mut self, e: &Expr) -> R<V> {
        self.type_level(e)?;
        self.eval_value(e)
    }

    pub(super) fn enter_branch(&mut self, c: &V, b: bool) {
        self.push();
        self.add_fact(if b {
            c.clone()
        } else {
            un("!", c.clone())
        });
    }

    // 実行時の検査を通常の if と構築子へ展開し、成功する枝で細分化条件を証明する。
    // ponytail: 基本型の条件式だけを扱う。データ型は条件式の再構築と証明の対応を足してから対応する。
    pub(super) fn refine_value(&mut self, e: &mut Expr) -> R<V> {
        // 条件の値を実行時の式にする。Var(0) は検査する値。ほかの変数は、その値を持つ見えている変数を探し、
        // 値を検査する前に let で写した名前（captures）を使う。値の式が mut の変数を書き換えても、条件は型を評価した時点の値で検査する
        fn predicate(v: &V, name: &Name, vars: &HashMap<(bool, usize), Name>, captures: &mut Vec<(Name, Name)>) -> R<Expr> {
            let mut go = |v: &V| predicate(v, name, vars, captures);
            Ok(match v {
                V::Int(n) => Expr::Int(*n), V::Num(n) => Expr::Num(*n), V::Bool(b) => Expr::Bool(*b), V::Str(s) => Expr::Str(s.to_string()),
                V::Neu(n) => match &**n {
                    Term::Var(0, _) => Expr::Var(name.into()),
                    Term::Var(..) | Term::Opaque(_) => match vars.get(&match &**n { Term::Opaque(k) => (true, *k), Term::Var(id, _) => (false, *id), _ => unreachable!() }) {
                        Some(var) => {
                            let capture = match captures.iter().find(|c| c.1 == *var) {
                                Some(c) => c.0.clone(),
                                None => {
                                    let c = Name::temporary("refine_capture", NodeId::default().raw() as usize);
                                    captures.push((c.clone(), var.clone()));
                                    c
                                }
                            };
                            Expr::Var(capture)
                        }
                        None => return Err("refine の条件の変数が見えない（実行時の式に変換できない）".into()),
                    },
                    Term::Num(v) => go(v)?,
                    Term::Un(op, v) => Expr::Unary(op, Box::new(go(v)?)),
                    Term::Bin(op, a, b) => { let a = go(a)?; Expr::Binary(op, Box::new(a), Box::new(go(b)?)) }
                    Term::Call(f, args) => Expr::Call(f.clone(), args.iter().map(go).collect::<R<_>>()?),
                    Term::Proj(v, field) => Expr::Field(Box::new(go(v)?), field.clone()),
                },
                _ => return Err("refine の条件を実行時の式に変換できない".into()),
            })
        }
        let Expr::Call(_, args) = e else { unreachable!() };
        if args.len() != 2 { return Err("refine<型>(値) と書く".into()); }
        let target = self.eval_type(&args[0])?;
        let (base, conditions) = split(&target);
        let V::Prim(kind @ ("int" | "number" | "String" | "bool")) = base else { return Err("refine は int・number・String・bool の細分化型に使う".into()) };
        let name = Name::temporary("refine", self.norm.fresh());
        // 変数の値の項から、その値を持つ見えている変数の名前。内側の宣言を優先する
        let mut vars = HashMap::new();
        for scope in &self.scopes {
            for (n, entry) in &scope.vars {
                if let V::Neu(t) = &entry.val {
                    match &**t {
                        Term::Var(id, _) => { vars.insert((false, *id), n.clone()); }
                        Term::Opaque(k) => { vars.insert((true, *k), n.clone()); }
                        _ => {}
                    }
                }
            }
        }
        let mut captures = Vec::new();
        let mut condition = Expr::Bool(true);
        for p in conditions { condition = Expr::Binary("&&", Box::new(condition), Box::new(predicate(&p, &name, &vars, &mut captures)?)); }
        let input_ty = self.expr(&mut args[1].clone())?;
        let narrowing = kind == "int" && super::base(&input_ty) == prim("number");
        let input_name = Name::temporary("refine_input", self.norm.fresh());
        let input = args.pop().unwrap();
        let value = if narrowing { Expr::Call("num_floor".into(), vec![Expr::Var(input_name.clone())]) } else { input.clone() };
        let expected = V::DataT(Rc::new(("std.Result".into(), vec![target, prim("String")])));
        let leaf = |value| Block { id: Default::default(), stmts: vec![], tail: Some(Box::new(value)) };
        let mut expanded = Expr::Block(Block { id: Default::default(),
            stmts: vec![Stmt::Let { name: name.clone().into(), mutable: false, ty: Some(Expr::Var(kind.into())), init: value }],
            tail: Some(Box::new(Expr::If(Box::new(condition), leaf(Expr::Call("std.Ok".into(), vec![Expr::Var(name.into())])), Some(Box::new(Expr::Block(leaf(Expr::Call("std.Err".into(), vec![Expr::Str("細分化型の条件を満たさない".into())])))))))),
        });
        if narrowing {
            let var = || Expr::Var(input_name.clone());
            let cmp = |op, a, b| Expr::Binary(op, Box::new(a), Box::new(b));
            // 範囲検査を先に行い、floor 自体が範囲外で停止するのを防ぐ。
            let bounds = cmp("&&", cmp("<=", Expr::Num(MIN_SAFE_INT as f64), var()), cmp("<=", var(), Expr::Num(MAX_SAFE_INT as f64)));
            let integral = cmp("==", Expr::Call("num_floor".into(), vec![var()]), var());
            let guard = cmp("&&", bounds, integral);
            expanded = Expr::Block(Block { id: Default::default(),
                stmts: vec![Stmt::Let { name: input_name, mutable: false, ty: Some(Expr::Var("number".into())), init: input }],
                tail: Some(Box::new(Expr::If(Box::new(guard), leaf(expanded), Some(Box::new(Expr::Block(leaf(Expr::Call("std.Err".into(), vec![Expr::Str("安全整数の範囲外か整数でない".into())])))))))),
            });
        }
        if !captures.is_empty() {
            let stmts = captures.into_iter().map(|(c, var)| Stmt::Let { name: c, mutable: false, ty: None, init: Expr::Var(var) }).collect();
            expanded = Expr::Block(Block { id: Default::default(), stmts, tail: Some(Box::new(expanded)) });
        }
        self.check(&mut expanded, &expected)?;
        *e = expanded;
        Ok(expected)
    }

}
