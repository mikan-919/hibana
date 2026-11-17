//! 効果の検査、ハンドラーと継続の制約。
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
            && fns
                .get(&n)
                .is_some_and(|g| g.op.as_deref() == Some(eff))
        {
            found = Some(*sp);
        }
    });
    found
}

pub(super) fn resume_tail(e: &Expr, tail: bool) -> R<()> {
    let block = |b: &Block, tail: bool| -> R<()> {
        for s in &b.stmts {
            match s {
                Stmt::Let { init, .. } => resume_tail(init, false)?,
                Stmt::Expr(e) => resume_tail(e, false)?,
            }
        }
        b.tail.as_ref().map_or(Ok(()), |t| resume_tail(t, tail))
    };
    match e {
        Expr::At(sp, x) => resume_tail(x, tail).map_err(|err| err.at(*sp)),
        Expr::Id(_, x) => resume_tail(x, tail),
        Expr::Call(n, args) if n == "resume" => {
            if !tail {
                return Err(
                    "resume は腕の最後に置く（続きの再開は腕の値になる位置でだけできる）".into(),
                );
            }
            args.iter().try_for_each(|a| resume_tail(a, false))
        }
        Expr::If(c, t, e) => {
            resume_tail(c, false)?;
            // else の無い if の値は捨てられる
            block(t, tail && e.is_some())?;
            e.as_ref().map_or(Ok(()), |e| resume_tail(e, tail))
        }
        Expr::Block(b) => block(b, tail),
        Expr::Match(s, arms) => {
            resume_tail(s, false)?;
            arms.iter().try_for_each(|a| resume_tail(&a.body, tail))
        }
        e => {
            let mut found = false;
            walk(e, &mut |x| {
                found |= matches!(x, Expr::Call(n, _) if n == "resume");
            });
            if found {
                return Err(
                    "resume は腕の最後に置く（続きの再開は腕の値になる位置でだけできる）".into(),
                );
            }
            Ok(())
        }
    }
}

// 式の中で代入される変数名を集める

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
                _ => return Err(format!("{} は効果でない", show(&v)).into()),
            }
        }
        Ok(out)
    }

    // 効果 inst を起こしてよいか確かめる。handle の本体の効果の型がまだ決まっていなければ、ここで決める
    pub(super) fn use_effect(&mut self, inst: V) -> R<()> {
        let key = eff_key(&inst);
        // ponytail: 腕は同期的に呼ぶので、腕の中では待てない。待つなら腕を async にして $op も待つ
        if key == "Async" && self.in_arm > 0 {
            return Err("handle の腕の中では Async の効果を起こせない".into());
        }
        let Some(slot) = self
            .avail
            .iter_mut()
            .rev()
            .find(|(n, _)| *n == key || n == "*")
        else {
            let s = show(&inst);
            return Err(format!(
                "効果 {s} を起こすには、関数の型に ! {s} を書くか、handle で処理する"
            )
            .into());
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
                return Err(format!("効果 {} が必要だが {}", show(have), show(&inst)).into());
            }
        }
        Ok(())
    }

    // handle 本体 { 操作(引数) => 式 }。本体では腕の効果を起こしてよい。腕は handle の外の効果を起こせる（Async は除く）
    // ponytail: resume は腕の末尾でだけ使える（続きを1回、その場で再開する）。末尾以外で再開するなら継続を実装する
    pub(super) fn handle(&mut self, body: &mut Expr, arms: &mut [Arm]) -> R<V> {
        for (i, a) in arms.iter().enumerate() {
            if arms[..i].iter().any(|b| b.ctor == a.ctor) {
                return Err(at_opt(
                    format!("操作 {} の腕が重複している", a.ctor).into(),
                    a.span,
                ));
            }
        }
        let mut effs: Vec<Name> = Vec::new();
        for a in &mut *arms {
            let Some(f) = self.norm.fns.get(&a.ctor).cloned() else {
                return Err(at_opt(format!("未定義の操作 {}", a.ctor).into(), a.span));
            };
            let Some(e) = &f.op else {
                return Err(at_opt(
                    format!("{} は効果の操作でない", a.ctor).into(),
                    a.span,
                ));
            };
            if a.binds.len() != f.params.len() - f.implicit {
                return Err(at_opt(
                    format!("操作 {} の引数は{}個", a.ctor, f.params.len() - f.implicit).into(),
                    a.span,
                ));
            }
            if !effs.contains(e) {
                effs.push(e.clone());
            }
            resume_tail(&a.body, true)?;
        }
        let mut ret = false;
        for e in std::iter::once(&*body).chain(arms.iter().map(|a| &a.body)) {
            walk(e, &mut |x| ret |= matches!(x, Expr::Return(_)));
        }
        if ret {
            return Err("handle の中では return を使えない".into());
        }
        let n = self.avail.len();
        self.avail.extend(effs.iter().map(|e| (e.clone(), None)));
        let t = self.expr(body);
        let insts = self.avail.split_off(n);
        let t = t?;
        self.in_arm += 1;
        let r = arms.iter_mut().try_for_each(|a| self.arm(a, &insts, &t));
        self.in_arm -= 1;
        r?;
        Ok(t)
    }

    // handle の腕を、handle の値の型 t で検査する。resume には操作の戻り値の型の値を渡す
    pub(super) fn arm(&mut self, a: &mut Arm, insts: &[(Name, Option<V>)], t: &V) -> R<()> {
        let f = self.norm.fns[&a.ctor].clone();
        let eff = f.op.as_ref().unwrap();
        let Some((_, Some(V::DataT(d)))) = insts.iter().find(|(n, _)| n == eff) else {
            return Err(format!("handle の本体が効果 {eff} を起こしていない").into());
        };
        let d = d.clone();
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
        r.ctx(|e| format!("操作 {} の腕: {e}", a.ctor))
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
        let Some((v, rest)) = async_of(&t) else {
            self.async_calls.remove(&target);
            let poly = self.var_awaits.contains(&target) || self.async_inst.contains(&target);
            return Err(format!(
                ".await は Async<T> の値にだけ付けられる（実際は {}）{}",
                show(&t),
                if poly { "。効果の変数を通した呼び出しは .await を書かずに待つ" } else { "" }
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
        let thunk = Expr::Lam(vec![], None, vec![], body);
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
