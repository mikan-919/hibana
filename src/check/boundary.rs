use super::*;

impl Eval {
    pub(super) fn check_boundary_signature(&mut self, f: &Fn) -> R<()> {
        let imported = f.js.is_some();
        let mut env: Env = vec![HashMap::new()];
        for (name, expr) in &f.params {
            let ty = self.eval_or_opaque(expr, &mut env)?;
            self.check_boundary_type(&ty, !imported, &mut Vec::new())
                .ctx(|e| format!("関数 {} の引数 {name}: {e}", f.name))?;
            env[0].insert(name.clone(), self.opaque());
        }
        if imported && let Some(expr) = &f.ret {
            let ty = self.eval_or_opaque(expr, &mut env)?;
            self.check_boundary_type(&ty, true, &mut Vec::new())
                .ctx(|e| format!("関数 {} の戻り値: {e}", f.name))?;
        }
        Ok(())
    }

    fn check_boundary_type(
        &mut self,
        ty: &V,
        incoming: bool,
        visiting: &mut Vec<(V, bool)>,
    ) -> R<()> {
        match ty {
            V::Refine(_) if incoming => Err(Error::from("外部から受け取る値に細分化型を使えない")
                .help("元の型で受け取り、refine<T> で条件を検査する".into())),
            V::Refine(r) => self.check_boundary_type(&r.0, incoming, visiting),
            V::VecT(item) => self.check_boundary_type(item, incoming, visiting),
            // 関数を渡す向きに対して、呼び出し時の引数は逆向きに流れる。
            V::Pi(f) => {
                for param in &f.doms {
                    self.check_boundary_type(param, !incoming, visiting)?;
                }
                self.check_boundary_type(&f.cod, incoming, visiting)
            }
            V::DataT(d) => {
                let Some(data) = self.datas.get(&d.0).cloned() else {
                    for arg in &d.1 {
                        self.check_boundary_type(arg, incoming, visiting)?;
                    }
                    return Ok(());
                };
                let key = (ty.clone(), incoming);
                if visiting.contains(&key) {
                    return Ok(());
                }
                // ponytail: 型引数が変わる再帰は64段で止める。必要なら型引数とフィールドの関係を先に解析する。
                if visiting.len() >= 64 {
                    return Err("境界でフィールドの型をたどる深さが上限を超えた".into());
                }
                visiting.push(key);
                for ctor in &data.ctors {
                    let mut env: Env = vec![
                        data.params
                            .iter()
                            .map(|(name, _)| name.clone())
                            .zip(d.1.iter().cloned())
                            .collect(),
                    ];
                    for (name, expr) in &ctor.fields {
                        let field = self.eval_or_opaque(expr, &mut env)?;
                        self.check_boundary_type(&field, incoming, visiting)
                            .ctx(|e| format!("{} のフィールド {name}: {e}", ctor.name))?;
                        env[0].insert(name.clone(), self.opaque());
                    }
                }
                visiting.pop();
                Ok(())
            }
            V::Neu(_) => Err("境界で受け渡す値の型を確定できない".into()),
            _ => Ok(()),
        }
    }
}

pub(crate) struct BoundaryCheck(Eval);

impl BoundaryCheck {
    pub(crate) fn new(program: &Program) -> Self {
        let mut norm = Eval::new();
        norm.load_declarations(program);
        Self(norm)
    }

    pub(crate) fn signature(&mut self, f: &Fn) -> R<()> {
        self.0
            .check_boundary_signature(f)
            .map_err(|error| at_opt(error, f.span).with_line())
    }
}
