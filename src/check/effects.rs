//! 要求（with）の検査。提供（with ブロック）と exit の規則、要求の推論
use super::*;

pub(super) fn eff_key(v: &V) -> Name {
    match v {
        V::DataT(d) => d.0.clone(),
        V::Neu(n) => match &**n {
            Term::Var(id, name) => Name::effect_parameter(name.clone(), *id),
            _ => show(v).into(),
        },
        _ => show(v).into(),
    }
}

// 効果の並び。効果の変数に入れた並び（Arr）は開く
pub(super) fn flat(effs: &[V]) -> Vec<V> {
    effs.iter()
        .flat_map(|e| match e {
            V::Arr(xs) => flat(xs),
            e => vec![e.clone()],
        })
        .collect()
}

pub(super) fn op_site(fns: &HashMap<Name, Fn>, f: &Fn, eff: &str) -> Option<Span> {
    let mut found = None;
    walk(&Expr::Block(f.body.clone()), &mut |x| {
        if let Expr::At(sp, inner) = x
            && let Expr::Call(n, _) = inner.peel()
            && found.is_none()
            && (fns.get(&n).is_some_and(|g| g.op.as_deref() == Some(eff))
                // 修飾なしの log(m) は、型検査の前の本体ではメソッドの名前のまま
                || fns.values().any(|g| g.op.as_deref() == Some(eff) && super::op_method(&g.name) == Some(n.local())))
        {
            found = Some(*sp);
        }
    });
    found
}


impl Ck {
    // 効果の型の式を評価する。Async は組み込みの効果
    // 効果の変数に入った並びは開くので、複数の効果になりうる
    pub(super) fn effect_inst(&mut self, e: &Expr, env: &mut Env) -> R<Vec<V>> {
        if let Expr::Var(n) = e.peel()
            && n == "Async"
        {
            return Ok(vec![V::DataT(Rc::new(("Async".into(), vec![])))]);
        }
        let v = self.norm.eval_or_opaque(e, env)?;
        let mut out = Vec::new();
        for v in flat(&[v]) {
            match &v {
                V::DataT(d) if self.effect_names.contains(&d.0) => out.push(v),
                // 効果の変数（<E: Effect>）
                V::Neu(n) if matches!(&**n, Term::Var(..)) => out.push(v),
                _ => return Err(format!("{} は要求でない（トレイト・Async・要求の変数を書く）", show(&v)).into()),
            }
        }
        Ok(out)
    }

    // 効果 inst を起こしてよいか確かめる。handle の本体の効果の型がまだ決まっていなければ、ここで決める
    pub(super) fn use_effect(&mut self, inst: V) -> R<()> {
        let key = eff_key(&inst);
        // 腕を待つのは、メソッドの宣言に with Async があるトレイトだけ（JavaScript では $aop で腕を待つ）
        if key == "Async" && self.in_arm > 0 {
            return Err("with で提供する関数の中では Async を使えない（トレイトのメソッドの宣言に with Async を書くと使える）".into());
        }
        let Some(slot) = self
            .avail
            .iter_mut()
            .rev()
            .find(|(n, _)| *n == key || n == "*")
        else {
            let s = show(&inst);
            let head = s.split('<').next().unwrap_or(&s).to_string();
            return Err(format!("要求 {s} を使うには、関数に with {s} を書くか、with {head}(...) {{ }} で提供する").into());
        };
        if slot.0 == "*" {
            let c = self.collected.as_mut().unwrap();
            if !c.iter().any(|x| same(x, &inst)) {
                c.push(inst);
            }
            return Ok(());
        }
        match &slot.1 {
            None => slot.1 = Some(inst),
            Some(have) if same(have, &inst) => {}
            Some(have) => {
                return Err(format!("要求 {} が必要だが {}", show(have), show(&inst)).into());
            }
        }
        Ok(())
    }

    // with T(値) { 本体 } を書き換えた Handle。本体では腕の要求を使える。腕は with の外の要求を使える。
    // Async を使えるのは、メソッドの宣言に with Async があるトレイトの腕だけ。腕の中の exit は、この with を抜ける
    pub(super) fn handle(&mut self, id: NodeId, body: &mut Expr, arms: &mut [Arm], expected: Option<&V>) -> R<V> {
        for (i, a) in arms.iter().enumerate() {
            if arms[..i].iter().any(|b| b.ctor == a.ctor) {
                return Err(at_opt(format!("{} の提供が重複している", a.ctor).into(), a.span));
            }
        }
        let mut effs: Vec<Name> = Vec::new();
        for a in &mut *arms {
            let f = self.norm.fns[&a.ctor].clone();
            let e = f.op.clone().expect("with の腕は要求の操作");
            if a.binds.len() != f.params.len() - f.implicit {
                return Err(at_opt(
                    format!("{} を提供する無名関数の引数は{}個", a.ctor, f.params.len() - f.implicit).into(),
                    a.span,
                ));
            }
            if !effs.contains(&e) {
                effs.push(e);
            }
        }
        let mut ret = false;
        for e in std::iter::once(&*body).chain(arms.iter().map(|a| &a.body)) {
            walk(e, &mut |x| ret |= matches!(x, Expr::Return(_)));
        }
        if ret {
            return Err("with で提供するブロックの中では return を使えない".into());
        }
        let n = self.avail.len();
        self.avail.extend(effs.iter().map(|e| (e.clone(), None)));
        let p = self.provided.len();
        self.provided.extend(effs.iter().cloned());
        let t = match expected {
            Some(x) => self.check(body, x).map(|_| x.clone()),
            None => self.expr(body),
        };
        self.provided.truncate(p);
        let mut insts = self.avail.split_off(n);
        let t = t?;
        // 本体が使わなかった要求も、型引数が無ければ型が決まる
        for (name, inst) in &mut insts {
            if inst.is_none() && self.norm.datas.get(name).is_some_and(|d| d.params.is_empty()) {
                *inst = Some(V::DataT(Rc::new((name.clone(), vec![]))));
            }
        }
        self.exits.push(Some((id, t.clone())));
        let r = arms.iter_mut().try_for_each(|a| {
            let waits = has_async(&self.norm.fns[&a.ctor].effects);
            self.in_arm += usize::from(!waits);
            let r = self.without_loop(|ck| ck.arm(a, &insts, &t, waits));
            self.in_arm -= usize::from(!waits);
            r
        });
        self.exits.pop();
        r?;
        Ok(t)
    }

    // with の腕を、with の値の型 t で検査する。#resume にはメソッドの戻り値の型の値を渡す。waits なら JavaScript で待つ腕にする
    pub(super) fn arm(&mut self, a: &mut Arm, insts: &[(Name, Option<V>)], t: &V, waits: bool) -> R<()> {
        let f = self.norm.fns[&a.ctor].clone();
        let eff = f.op.as_ref().unwrap();
        let Some((_, Some(V::DataT(d)))) = insts.iter().find(|(n, _)| n == eff) else {
            return Err(at_opt(format!("with の本体が要求 {eff} を使っていない（型引数が決まらない）").into(), a.span));
        };
        let d = d.clone();
        if waits && let Expr::Block(b) = &a.body {
            self.awaits.insert(b.id);
        }
        self.push();
        let r = (|| {
            let mut fenv: Env = vec![HashMap::new()];
            for ((p, _), v) in f.params[..f.implicit].iter().zip(&d.1) {
                fenv[0].insert(p.clone(), v.clone());
            }
            for ((p, pt), b) in f.params[f.implicit..].iter().zip(&a.binds) {
                let ty = self.norm.eval_or_opaque(pt, &mut fenv)?;
                let id = self.norm.fresh();
                let v = neu(Term::Var(id, b.clone()));
                self.bind(b, ty, v.clone(), false)?;
                fenv[0].insert(p.clone(), v);
            }
            let rt = match &f.ret {
                Some(t) => self.norm.eval_or_opaque(t, &mut fenv)?,
                None => prim("unit"),
            };
            self.resume.push(rt);
            let r = self.check(&mut a.body, t);
            self.resume.pop();
            r
        })();
        self.pop();
        r.ctx(|e| format!("{} の提供: {e}", a.ctor))
            .map_err(|e| at_opt(e, a.span))
    }
    // トップレベルの関数を値として使った時の型
    // 無名関数の本体が効果 e を起こせるなら、JavaScript で待つ本体として記録する
    pub(super) fn note_body(&mut self, e: &V, body: &Block) {
        let k = eff_key(e);
        if k == "Async" {
            self.awaits.insert(body.id);
        }
        if k.is_effect_parameter() {
            self.var_awaits.insert(body.id);
        }
    }

    // e.await。e は Async<T> の値で、式の型は T。Async と、値が持つ残りの効果を起こす。
    // 直下が Async<T> を作る呼び出しなら、その呼び出しをその場で待つ。それ以外は値（引数なしの関数）を呼んで待つ
    pub(super) fn await_(&mut self, e: &mut Expr, addr: NodeId) -> R<V> {
        let Expr::Await(x) = e else { unreachable!() };
        let mut inner: &mut Expr = x;
        while let Expr::At(_, y) = inner {
            inner = y;
        }
        let (target, _) = inner.identified();
        let saved = self.direct_await.replace(target);
        let t = self.expr(x);
        self.direct_await = saved;
        let t = t?;
        // 推論の途中では、呼んだ関数が Async を持つとまだ分かっていないことがある。.await を書いたので Async を起こすとみなす
        if self.inferring && async_of(&t).is_none() {
            self.use_effect(async_t_effect())?;
            return Ok(t);
        }
        let Some((v, rest)) = async_of(&t) else {
            self.async_calls.remove(&target);
            let poly = self.var_awaits.contains(&target) || self.async_inst.contains(&target);
            return Err(format!(
                ".await は Async<T> の値にだけ付けられる（実際は {}）{}",
                show(&t),
                if poly { "。要求の変数を通した呼び出しは .await を書かずに待つ" } else { "" }
            )
            .into());
        };
        self.use_effect(async_t_effect())?;
        for r in rest {
            self.use_effect(r)?;
        }
        let x = std::mem::replace(&mut **x, Expr::Int(0));
        *e = if self.async_calls.remove(&target) {
            x
        } else {
            self.awaits.insert(addr);
            Expr::Apply(Box::new(x), vec![])
        };
        Ok(v)
    }

    // 呼び出し id が Async<T> の値を作り、.await の直下でなければ、引数をその場で評価して閉じ込めた、引数なしの関数にする。
    // 本体はその関数を呼んだ時（.await と合流の時）に初めて走る
    pub(super) fn delay(&mut self, id: NodeId, e: &mut Expr) {
        if self.direct_await == Some(id) || !self.async_calls.remove(&id) {
            return;
        }
        let mut stmts = Vec::new();
        let call = match std::mem::replace(e, Expr::Int(0)) {
            Expr::Call(n, args) => {
                // 明示した型引数は値でないので、そのまま渡す
                let skip = self.norm.fns.get(&n).filter(|f| args.len() == f.params.len()).map_or(0, |f| f.implicit);
                Expr::Call(n, self.hold(args, skip, &mut stmts))
            }
            Expr::Apply(f, args) => {
                let f = self.hold(vec![*f], 0, &mut stmts).remove(0);
                Expr::Apply(Box::new(f), self.hold(args, 0, &mut stmts))
            }
            other => unreachable!("Async<T> を作るのは呼び出しだけ: {other:?}"),
        };
        let body = Block { id: NodeId::default(), stmts: vec![], tail: Some(Box::new(Expr::Id(id, Box::new(call)))) };
        self.awaits.insert(body.id);
        let thunk = Expr::Lam(vec![], None, None, body);
        *e = Expr::Block(Block { id: NodeId::default(), stmts, tail: Some(Box::new(thunk)) });
    }

    // 引数を一時変数に置き、その変数の並びを返す。変数とリテラルはそのまま
    fn hold(&mut self, args: Vec<Expr>, skip: usize, stmts: &mut Vec<Stmt>) -> Vec<Expr> {
        let mut out = Vec::new();
        for (i, a) in args.into_iter().enumerate() {
            if i < skip || matches!(a.peel(), Expr::Var(_) | Expr::Int(_) | Expr::Num(_) | Expr::Bool(_) | Expr::Str(_)) {
                out.push(a);
                continue;
            }
            let tmp = Name::temporary("arg", self.norm.fresh());
            stmts.push(Stmt::Let { name: tmp.clone(), mutable: false, ty: None, init: a });
            out.push(Expr::Var(tmp));
        }
        out
    }
}

// 効果の Async
pub(super) fn async_t_effect() -> V {
    V::DataT(Rc::new(("Async".into(), vec![])))
}

// 効果の型の値を、関数の宣言に書く形の式に戻す。戻せない形（関数型・細分化型など）は None
// ponytail: 関数型と細分化型を引数に持つ効果は宣言に戻せず、推論から落ちる（本当の検査でその効果のエラーになる）。戻すには Pi と Refine も式に戻す
fn quote_type(v: &V) -> Option<Expr> {
    Some(match v {
        V::Prim(p) => Expr::Var(Name::from(*p)),
        V::DataT(d) if d.1.is_empty() => Expr::Var(d.0.clone()),
        V::DataT(d) => Expr::Call(d.0.clone(), d.1.iter().map(quote_type).collect::<Option<_>>()?),
        V::VecT(a) => Expr::Call(Name::from("Vec"), vec![quote_type(a)?]),
        V::Neu(n) => match &**n {
            Term::Var(_, name) => Expr::Var(name.clone()),
            _ => return None,
        },
        _ => return None,
    })
}

// with を省いた非公開関数の効果を、本体から推論して宣言に書き込む。
// 本体の複製を検査して起きた効果を集め、変わらなくなるまで繰り返す（再帰と相互再帰の不動点）。
// ponytail: 毎回すべての対象を検査し直し、回数は 8 回まで。遅くなれば呼び出しの依存の順に、変わった関数の呼び出し元だけを検査する
pub(super) fn infer_effects(ck: &mut Ck, prog: &mut Program) {
    let targets: Vec<usize> = (0..prog.fns.len())
        .filter(|&i| {
            let f = &prog.fns[i];
            !f.public && !f.effects_explicit && f.effects.is_empty()
                && f.op.is_none() && f.js.is_none() && !f.alias && !is_impl(&f.name)
        })
        .collect();
    ck.inferring = true;
    for _ in 0..8 {
        let mut changed = false;
        for &i in &targets {
            let mut g = prog.fns[i].clone();
            let _ = check_fn(ck, &mut g);
            let mut found = ck.collected.take().unwrap_or_default();
            if g.name == "main" && ck.avail.iter().any(|(k, v)| k == "Async" && v.is_some()) {
                found.push(async_t_effect());
            }
            let mut effs: Vec<(String, Expr)> = Vec::new();
            for v in flat(&found) {
                let key = show(&v);
                if let Some(e) = quote_type(&v)
                    && !effs.iter().any(|(k, _)| *k == key)
                {
                    effs.push((key, e));
                }
            }
            effs.sort_by(|a, b| a.0.cmp(&b.0));
            let shown = |es: &[Expr]| es.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>();
            let effs: Vec<Expr> = effs.into_iter().map(|(_, e)| e).collect();
            if shown(&effs) != shown(&prog.fns[i].effects) {
                changed = true;
                prog.fns[i].effects = effs;
                ck.norm.fns.insert(prog.fns[i].name.clone(), prog.fns[i].clone());
            }
        }
        if !changed {
            break;
        }
    }
    ck.inferring = false;
    ck.collected = None;
    // 推論のための検査で記録したものは捨て、本当の検査で記録し直す
    ck.awaits.clear();
    ck.var_awaits.clear();
    ck.async_inst.clear();
    ck.async_calls.clear();
}

// 効果の節に Async を書いたか
pub(super) fn has_async(effs: &[Expr]) -> bool {
    effs.iter().any(|e| matches!(e.peel(), Expr::Var(n) if n == "Async"))
}
