//! トレイトの実装探索と型引数の照合。
use super::*;

// impl から作った、実装の値を作る関数（Show#impl0。メソッドの Show#impl0#show は除く）
pub(super) fn is_impl(name: &str) -> bool {
    let last = name.rsplit('.').next().unwrap_or(name);
    let mut parts = last.split('#');
    parts.next();
    parts.next().is_some_and(|p| p.starts_with("impl")) && parts.next().is_none()
}


impl Ck {
    // == と to_str・put は、std の Eq・Show の実装がある型ならその実装を使う（無ければ組み込みの比較と文字列化）
    // プリミティブの型は組み込みのまま（細分化型の検査が int の == を直接扱うため）
    pub(super) fn trait_inst(&mut self, method: &str, t: &V) -> R<Option<Expr>> {
        let tr = if method == "eq" { "std.Eq" } else { "std.Display" };
        // Vec の表示は組み込みのまま（文字列の要素を引用符で囲む）。Show<Vec<A>> は制約で求められた時だけ使う
        let builtin_vec = method != "eq" && matches!(base(t), V::VecT(_));
        if builtin_vec || matches!(base(t), V::Prim(_)) || !self.norm.datas.contains_key(tr) {
            return Ok(None);
        }
        let goal = V::DataT(Rc::new((tr.into(), vec![base(t)])));
        match self.resolve_inst(&goal, 0) {
            Ok(inst) => Ok(Some(inst)),
            Err(_) => Ok(None),
        }
    }

    // トレイトの実装を探す。引数の実装（inst#k）とその親、impl の順
    // ponytail: impl が重なれば名前の順で最初のもの。重なりの検査は孤児ルールと一緒に足す
    pub(super) fn resolve_inst(&mut self, goal: &V, depth: usize) -> R<Expr> {
        if depth > 16 {
            return Err(format!("{} の実装を探すのが終わらない", show(goal)).into());
        }
        let mut cands: Vec<Expr> = self
            .scopes
            .iter()
            .flat_map(|s| s.vars.keys())
            .filter(|k| k.starts_with("inst#"))
            .map(|k| Expr::Var(k.clone()))
            .collect();
        let mut i = 0;
        while i < cands.len() {
            let mut c = cands[i].clone();
            let t = self.expr(&mut c)?;
            if self.unify(&t, goal, &[], &mut HashMap::new())? {
                return Ok(c);
            }
            if let V::DataT(d) = base(&t)
                && let Some(data) = self.norm.datas.get(d.0.as_str())
            {
                for (fname, _) in data.ctors.iter().flat_map(|c| &c.fields) {
                    if fname.starts_with("super#") {
                        cands.push(Expr::Field(Box::new(c.clone()), fname.clone()));
                    }
                }
            }
            i += 1;
        }
        for f in self.impls.clone() {
            let mut fenv: Env = vec![HashMap::new()];
            let mut ids = Vec::new();
            for (p, _) in &f.params[..f.implicit] {
                let id = self.norm.fresh();
                ids.push(id);
                fenv[0].insert(p.clone(), neu(Term::Var(id, format!("?{p}"))));
            }
            let rt = self
                .norm
                .eval_or_opaque(f.ret.as_ref().unwrap(), &mut fenv)?;
            let mut sol = HashMap::new();
            if !self.unify(&rt, goal, &ids, &mut sol)? {
                continue;
            }
            for ((p, _), id) in f.params[..f.implicit].iter().zip(&ids) {
                if let Some(v) = sol.get(id).cloned() {
                    let v = self.apply(&sol, &v)?;
                    fenv[0].insert(p.clone(), v);
                }
            }
            let mut subs = Vec::new();
            for (_, pt) in &f.params[f.implicit..] {
                let t = self.norm.eval_or_opaque(pt, &mut fenv)?;
                let t = self.apply(&sol, &t)?;
                subs.push(self.resolve_inst(&t, depth + 1)?);
            }
            return Ok(Expr::Call(f.name.clone(), subs));
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
                // 効果の変数が1つなら、相手の効果のうち残りをその値にする
                let (xe, ye) = (flat(&x.effs), flat(&y.effs));
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
            _ => same(&a, &b),
        })
    }

    // 構築子の呼び出し。型の引数はメタ変数にして、期待する型と引数の型から求める
}
