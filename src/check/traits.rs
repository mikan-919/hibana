//! トレイトの実装探索と型引数の照合。
use super::*;

// impl から作った、実装の値を作る関数（Show#impl0。メソッドの Show#impl0#show は除く）
pub(super) fn is_impl(name: &Name) -> bool { name.is_implementation() }

// 両方の実装に共通する型引数があるか、正規化した型の構造を照合する。
fn overlapping_heads(norm: &mut Eval, a: &V, b: &V, variables: &[usize]) -> R<bool> {
    let mut pairs = vec![(a.clone(), b.clone())];
    while let Some((a, b)) = pairs.pop() {
        if a == b {
            continue;
        }
        let binding = [(&a, &b), (&b, &a)].into_iter().find_map(|(x, y)| match x {
            V::Neu(n) => match &**n {
                Term::Var(id, _) if variables.contains(id) => Some((*id, y)),
                _ => None,
            },
            _ => None,
        });
        if let Some((id, value)) = binding {
            // T = Vec<T> のように、解に自分自身を含む場合は一致しない。
            if mentions(value, id) {
                return Ok(false);
            }
            for (a, b) in &mut pairs {
                *a = norm.subst(a, id, value)?;
                *b = norm.subst(b, id, value)?;
            }
            continue;
        }
        match (a, b) {
            (V::DataT(a), V::DataT(b)) | (V::Data(a), V::Data(b))
                if a.0 == b.0 && a.1.len() == b.1.len() =>
            {
                pairs.extend(a.1.iter().cloned().zip(b.1.iter().cloned()));
            }
            (V::VecT(a), V::VecT(b)) => pairs.push(((*a).clone(), (*b).clone())),
            (V::Refine(a), V::Refine(b)) => {
                pairs.push((a.0.clone(), b.0.clone()));
                pairs.push((a.1.clone(), b.1.clone()));
            }
            (V::Pi(a), V::Pi(b)) if a.doms.len() == b.doms.len() => {
                let (mut ae, mut be) = (flat(&a.effs), flat(&b.effs));
                if ae.len() != be.len() {
                    return Ok(false);
                }
                ae.sort_by_key(eff_key);
                be.sort_by_key(eff_key);
                pairs.extend(ae.into_iter().zip(be));
                pairs.extend(a.doms.iter().cloned().zip(b.doms.iter().cloned()));
                pairs.push((a.cod.clone(), b.cod.clone()));
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn check_equality_type(t: &V) -> R<()> {
    if matches!(t, V::Pi(_) | V::Prim("syn") | V::Syn(_))
        || matches!(t, V::DataT(d) if d.0 == "Async" && !d.1.is_empty())
    {
        return Err(format!("{} は比較できない", show(t)).into());
    }
    Ok(())
}

fn check_eq_goal(t: &V) -> R<()> {
    if let V::DataT(d) = t
        && d.0 == "std.Eq"
        && let [target] = d.1.as_slice()
    {
        check_equality_type(&base(target))?;
    }
    Ok(())
}

impl Ck {
    fn impl_head(&mut self, f: &Fn) -> R<(Vec<usize>, Env, V)> {
        let mut env: Env = vec![HashMap::new()];
        let mut ids = Vec::new();
        for (p, _) in &f.params[..f.implicit] {
            let id = self.norm.fresh();
            ids.push(id);
            env[0].insert(p.clone(), neu(Term::Var(id, format!("?{p}").into())));
        }
        let head = self.norm.eval_or_opaque(f.ret.as_ref().unwrap(), &mut env)?;
        Ok((ids, env, head))
    }

    pub(super) fn check_impl_overlaps(&mut self, fns: &[Fn]) -> R<()> {
        let mut heads: HashMap<Name, Vec<(V, Vec<usize>, Option<Span>)>> = HashMap::new();
        for f in fns.iter().filter(|f| is_impl(&f.name)) {
            // ponytail: 制約と値引数の充足は調べない。対象を広げるには制約と型の計算を解く必要がある。
            if f.insts != 0 || f.params[..f.implicit].iter().any(|(_, ty)| {
                !matches!(ty.peel(), Expr::Var(n) if n == "Type")
            }) {
                continue;
            }
            let (ids, _, head) = self.impl_head(f).map_err(|e| at_opt(e, f.span))?;
            let V::DataT(data) = &head else { continue };
            let same_trait = heads.entry(data.0.clone()).or_default();
            for (other, other_ids, span) in same_trait.iter() {
                let variables: Vec<_> = ids.iter().chain(other_ids).copied().collect();
                if overlapping_heads(&mut self.norm, &head, other, &variables).map_err(|e| at_opt(e, f.span))? {
                    let mut error = Error::from(format!(
                        "トレイト実装が重複している（{} と {}）", show(other), show(&head)
                    ));
                    if let Some(span) = span {
                        error.origin = Some((span.start.offset, span.end.offset - span.start.offset));
                        error.origin_source = Some(span.source);
                        error.origin_label = Some("重複する実装の宣言");
                    }
                    return Err(at_opt(error, f.span));
                }
            }
            same_trait.push((head, ids, f.span));
        }
        Ok(())
    }

    // 組み込みの比較はフィールドや配列要素の Eq を呼ばない。
    pub(super) fn check_builtin_equality(&mut self, t: &V, visiting: &mut Vec<V>) -> R<()> {
        let t = base(t);
        check_equality_type(&t)?;
        match &t {
            V::Neu(_) => Err(format!(
                "型 {} の組み込みの比較を決められない（比較する型に Eq の実装が必要）",
                show(&t)
            )
            .into()),
            V::VecT(element) => self.check_builtin_equality(element, visiting),
            V::DataT(d) if d.0 != "Ref" && d.0 != "Task" => {
                if visiting.contains(&t) {
                    return Ok(());
                }
                let Some(data) = self.norm.datas.get(&d.0).cloned() else {
                    return Ok(());
                };
                // ponytail: 型引数が変わり続ける再帰は64段で止める。扱うならフィールドが使う型引数を別に解析する。
                if visiting.len() >= 64 {
                    return Err("組み込みの比較でフィールドの型をたどる深さが上限を超えた".into());
                }
                visiting.push(t.clone());
                for ctor in &data.ctors {
                    let mut env = vec![data.params.iter()
                        .map(|(n, _)| n.clone())
                        .zip(d.1.iter().cloned())
                        .collect()];
                    for (field, ty) in &ctor.fields {
                        let ty = self.norm.eval_or_opaque(ty, &mut env)?;
                        self.check_builtin_equality(&ty, visiting)
                            .ctx(|e| format!("{} のフィールド {field}: {e}", ctor.name))?;
                        env[0].insert(field.clone(), self.norm.opaque());
                    }
                }
                visiting.pop();
                Ok(())
            }
            _ => Ok(()),
        }
    }

    // == と to_str・put は、std の Eq・Show の実装がある型ならその実装を使う（無ければ組み込みの比較と文字列化）
    // プリミティブの型は組み込みのまま（細分化型の検査が int の == を直接扱うため）
    pub(super) fn trait_inst(&mut self, method: &str, t: &V) -> R<Option<Expr>> {
        let tr = if method == "eq" { "std.Eq" } else { "std.Display" };
        // Vec の表示は組み込みのまま（文字列の要素を引用符で囲む）。Show<Vec<A>> は制約で求められた時だけ使う
        let builtin_vec = method != "eq" && matches!(base(t), V::VecT(_));
        if builtin_vec || matches!(base(t), V::Prim(_)) || !self.norm.datas.contains_key(&Name::from(tr)) {
            return Ok(None);
        }
        let goal = V::DataT(Rc::new((tr.into(), vec![base(t)])));
        match self.resolve_inst(&goal, 0) {
            Ok(inst) => Ok(Some(inst)),
            Err(_) => Ok(None),
        }
    }

    // トレイトの実装を探す。引数の実装（inst#k）とその親、impl の順
    // ponytail: 制約付き impl の重複は検査していない。適用条件の重なりが判定できるようになったら宣言時に検査する。
    pub(super) fn resolve_inst(&mut self, goal: &V, depth: usize) -> R<Expr> {
        self.resolve_in(goal, depth, &[]).map(|r| r.0)
    }

    // goal の中のメタ変数 metas は、実装に合わせて決めてよい（トレイトの型引数が分からない時のメソッドの探索）。見つけた実装の型も返す
    pub(super) fn resolve_in(&mut self, goal: &V, depth: usize, metas: &[usize]) -> R<(Expr, V)> {
        check_eq_goal(goal)?;
        if depth > 16 {
            return Err(format!("{} の実装を探すのが終わらない", show(goal)).into());
        }
        let mut cands: Vec<Expr> = self
            .scopes
            .iter()
            .flat_map(|s| s.vars.keys())
            .filter(|k| k.is_instance())
            .map(|k| Expr::Var(k.clone()))
            .collect();
        let mut i = 0;
        while i < cands.len() {
            let mut c = cands[i].clone();
            let t = self.expr(&mut c)?;
            if self.unify(&t, goal, metas, &mut HashMap::new())? && check_eq_goal(&t).is_ok() {
                return Ok((c, t));
            }
            if let V::DataT(d) = base(&t)
                && let Some(data) = self.norm.datas.get(&d.0)
            {
                for (fname, _) in data.ctors.iter().flat_map(|c| &c.fields) {
                    if fname.is_supertrait() {
                        cands.push(Expr::Field(Box::new(c.clone()), fname.clone()));
                    }
                }
            }
            i += 1;
        }
        for f in self.impls.clone() {
            let (mut ids, mut fenv, rt) = self.impl_head(&f)?;
            let mut sol = HashMap::new();
            ids.extend_from_slice(metas);
            if !self.unify(&rt, goal, &ids, &mut sol)? {
                continue;
            }
            for ((p, _), id) in f.params[..f.implicit].iter().zip(&ids) {
                if let Some(v) = sol.get(id).cloned() {
                    let v = self.apply(&sol, &v)?;
                    fenv[0].insert(p.clone(), v);
                }
            }
            let rt = self.apply(&sol, &rt)?;
            if check_eq_goal(&rt).is_err() {
                continue;
            }
            let mut subs = Vec::new();
            for (_, pt) in &f.params[f.implicit..] {
                let t = self.norm.eval_or_opaque(pt, &mut fenv)?;
                let t = self.apply(&sol, &t)?;
                subs.push(self.resolve_inst(&t, depth + 1)?);
            }
            return Ok((Expr::Call(f.name.clone(), subs), rt));
        }
        Err(format!("{} の実装が無い（impl で書く）", show(goal)).into())
    }

    pub(super) fn apply(&mut self, sol: &HashMap<usize, V>, v: &V) -> R<V> {
        let mut v = v.clone();
        // 解が別の解を含むことがあるので2回当てる
        for _ in 0..2 {
            for (id, x) in sol {
                v = self.norm.subst(&v, *id, x)?;
            }
        }
        Ok(v)
    }

    // a と b を等しくするメタ変数の解を sol に足す。等しくできなければ false
    pub(super) fn unify(&mut self, a: &V, b: &V, metas: &[usize], sol: &mut HashMap<usize, V>) -> R<bool> {
        let (a, b) = (self.apply(sol, a)?, self.apply(sol, b)?);
        if a == b {
            return Ok(true);
        }
        for (x, y) in [(&a, &b), (&b, &a)] {
            if let V::Neu(n) = x
                && let Term::Var(id, _) = &**n
                && metas.contains(id)
            {
                sol.insert(*id, y.clone());
                return Ok(true);
            }
        }
        Ok(match (&a, &base(&b)) {
            (V::DataT(x), V::DataT(y)) | (V::Data(x), V::Data(y))
                if x.0 == y.0 && x.1.len() == y.1.len() =>
            {
                for (p, q) in x.1.iter().zip(&y.1) {
                    if !self.unify(p, q, metas, sol)? {
                        return Ok(false);
                    }
                }
                true
            }
            (V::VecT(x), V::VecT(y)) => self.unify(x, y, metas, sol)?,
            (V::Pi(x), V::Pi(y)) if x.doms.len() == y.doms.len() => {
                for (p, q) in x.doms.iter().zip(&y.doms).chain([(&x.cod, &y.cod)]) {
                    if !self.unify(p, q, metas, sol)? {
                        return Ok(false);
                    }
                }
                // 同じ要求（Fail<?E> と Fail<String>）は型引数を合わせる。効果の変数が1つなら、相手の効果のうち残りをその値にする
                let (xe, mut ye) = (flat(&x.effs), flat(&y.effs));
                for e in &xe {
                    if let V::DataT(d) = e
                        && let Some(k) = ye.iter().position(|y| matches!(y, V::DataT(o) if o.0 == d.0))
                    {
                        if !self.unify(e, &ye[k], metas, sol)? {
                            return Ok(false);
                        }
                        ye.remove(k);
                    }
                }
                let xe: Vec<V> = xe.into_iter().filter(|e| !matches!(e, V::DataT(_))).collect();
                let vars: Vec<usize> = xe
                    .iter()
                    .filter_map(|e| match e {
                        V::Neu(n) => match &**n {
                            Term::Var(id, _) if metas.contains(id) && !sol.contains_key(id) => {
                                Some(*id)
                            }
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect();
                if let [id] = vars.as_slice() {
                    let rest: Vec<V> = ye.into_iter().filter(|e| !xe.contains(e)).collect();
                    sol.insert(*id, V::Arr(Rc::new(rest)));
                }
                true
            }
            // 部分型への数値変換は式の検査で行う。number の実装を int の辞書に流用しない。
            _ => same(&a, &b) && !(base(&a) == prim("number") && base(&b) == prim("int")),
        })
    }

    // 構築子の呼び出し。型の引数はメタ変数にして、期待する型と引数の型から求める
}
