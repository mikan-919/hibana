//! match の腕と let の分解に書くパターン。入れ子のパターンは構文解析で、平らな match（構築子と束縛だけの腕）・if・let に書き換える。
//! 型検査とバックエンドは平らな形だけを扱う。構築子の網羅は型検査の match の検査に任せ、リテラルの網羅はここで調べる
use super::parser::{Arm, Block, Expr, Name, NameKind, NodeId, Parser, Stmt, R};
use crate::lexer::{Span, Tok};

#[derive(Debug, Clone)]
pub(super) enum Pat {
    Wild,
    Bind(Name),
    Lit(Expr),
    // 構築子とフィールドのパターン。構造体のパターンはフィールドを宣言の順に並べ、書いていないフィールドは Wild
    Ctor(Name, Vec<Pat>),
}

// 照合する行。pats は occs（照合する値を置いた変数）と同じ並び。binds は既に決まった束縛
struct Row {
    pats: Vec<Pat>,
    binds: Vec<(Name, Name)>,
    body: Expr,
}

impl Row {
    // col 番目を subs に置き換える。col 番目が束縛なら、その値の変数に束縛する
    fn expand(&self, col: usize, subs: Vec<Pat>, occ: &Name) -> Row {
        let mut pats = self.pats[..col].to_vec();
        pats.extend(subs);
        pats.extend(self.pats[col + 1..].iter().cloned());
        let mut binds = self.binds.clone();
        if let Pat::Bind(x) = &self.pats[col] {
            binds.push((x.clone(), occ.clone()));
        }
        Row { pats, binds, body: self.body.clone() }
    }
}

fn wild(p: &Pat) -> bool {
    matches!(p, Pat::Wild | Pat::Bind(_))
}

fn new_var(template: &str) -> Name {
    Name::pattern(template, NodeId::default().raw() as usize)
}

fn template(occ: &Name) -> String {
    match &occ.kind {
        NameKind::Pattern(t, _) => t.clone(),
        _ => "□".into(),
    }
}

fn leaf(e: Expr) -> Block {
    Block { id: Default::default(), stmts: vec![], tail: Some(Box::new(e)) }
}

fn ctor_label(c: &Name) -> String {
    c.structure_owner().unwrap_or_else(|| c.clone()).to_string()
}

impl Parser<'_> {
    // top は腕の先頭と let。先頭の名前だけのパターン（Nil）は構築子、入れ子の中ではフィールドの無い構築子でなければ束縛
    pub(super) fn value_pattern(&mut self, top: bool) -> R<Pat> {
        let start = self.pos;
        let negative = self.peek() == &Tok::Punct("-");
        if negative {
            self.pos += 1;
        }
        match self.peek().clone() {
            Tok::Int(n) => {
                self.pos += 1;
                return Ok(Pat::Lit(self.wrap(start, Expr::Int(if negative { -n } else { n }))));
            }
            Tok::Float(n) => {
                self.pos += 1;
                return Ok(Pat::Lit(self.wrap(start, Expr::Num(if negative { -n } else { n }))));
            }
            _ if negative => return self.err("- の後には数が必要"),
            Tok::Str(s) => {
                self.pos += 1;
                return Ok(Pat::Lit(self.wrap(start, Expr::Str(s))));
            }
            Tok::Punct(b @ ("true" | "false")) => {
                self.pos += 1;
                return Ok(Pat::Lit(Expr::Bool(b == "true")));
            }
            Tok::Ident(n) if n == "_" => {
                self.pos += 1;
                return Ok(Pat::Wild);
            }
            Tok::Ident(_) => {}
            _ => return self.err("パターンが必要"),
        }
        self.qualify_here()?;
        let n = self.ident()?;
        let structure = self.structs.get(&self.qualify(&n)).cloned();
        if !self.at_newline() && self.peek() == &Tok::Punct("{") && let Some(s) = &structure {
            self.pos += 1;
            let mut pats = vec![Pat::Wild; s.fields.len()];
            while !self.eat("}") {
                let f = self.ident()?;
                let Some(k) = s.fields.iter().position(|x| *x == f) else {
                    self.pos -= 1;
                    return self.err_at(format!("構造体 {n} にフィールド {f} は無い"));
                };
                pats[k] = if self.eat(":") { self.value_pattern(false)? } else { Pat::Bind(f) };
                if !self.eat(",") {
                    self.expect("}")?;
                    break;
                }
            }
            return Ok(Pat::Ctor(s.constructor.clone(), pats));
        }
        if !self.at_newline() && self.eat("(") {
            let mut pats = Vec::new();
            while !self.eat(")") {
                pats.push(self.value_pattern(false)?);
                if !self.eat(",") {
                    self.expect(")")?;
                    break;
                }
            }
            return Ok(Pat::Ctor(structure.map_or(n, |s| s.constructor), pats));
        }
        if top || self.nullary.contains(&n) || self.nullary.contains(&self.qualify(&n)) {
            Ok(Pat::Ctor(n, vec![]))
        } else {
            Ok(Pat::Bind(n))
        }
    }

    // match 対象 { パターン -> 本体 ... }。腕が平らなら今までの形の match にし、そうでなければ決定木に書き換える
    // ponytail: 束縛や _ を含む腕は、それに続く分岐ごとに本体を複製する。腕が多く深いと出力が大きくなる。腕の番号で分岐し直せば複製は要らない
    pub(super) fn compile_match(&mut self, scrutinee: Expr, arms: Vec<(Option<Span>, Pat, Expr)>) -> R<Expr> {
        let flat = arms.iter().all(|(_, p, _)| match p {
            Pat::Wild => true,
            Pat::Ctor(_, xs) => xs.iter().all(wild),
            _ => false,
        });
        if flat {
            let arms = arms
                .into_iter()
                .map(|(span, p, body)| match p {
                    Pat::Ctor(ctor, xs) => {
                        let binds = xs.into_iter().map(|x| if let Pat::Bind(n) = x { n } else { "_".into() }).collect();
                        Arm { span, ctor, binds, body }
                    }
                    _ => Arm { span, ctor: "_".into(), binds: vec![], body },
                })
                .collect();
            return Ok(Expr::Match(Box::new(scrutinee), arms));
        }
        let root = new_var("□");
        let rows = arms.into_iter().map(|(_, p, body)| Row { pats: vec![p], binds: vec![], body }).collect();
        let tree = self.decide(vec![root.clone()], rows)?;
        Ok(Expr::Block(Block {
            id: Default::default(),
            stmts: vec![Stmt::Let { name: root, mutable: false, ty: None, init: scrutinee }],
            tail: Some(Box::new(tree)),
        }))
    }

    // 行の並びを、最初の行が決まる位置から順に分ける（Maranget の方法を簡単にしたもの）
    fn decide(&mut self, occs: Vec<Name>, rows: Vec<Row>) -> R<Expr> {
        let Some(col) = rows[0].pats.iter().position(|p| !wild(p)) else {
            let row = rows.into_iter().next().unwrap();
            let mut stmts = Vec::new();
            for (x, o) in row.binds.iter().cloned().chain(row.pats.iter().zip(&occs).filter_map(|(p, o)| match p {
                Pat::Bind(x) => Some((x.clone(), o.clone())),
                _ => None,
            })) {
                stmts.push(Stmt::Let { name: x, mutable: false, ty: None, init: Expr::Var(o) });
            }
            return Ok(if stmts.is_empty() { row.body } else { Expr::Block(Block { id: Default::default(), stmts, tail: Some(Box::new(row.body)) }) });
        };
        let occ = occs[col].clone();
        let shape = template(&occ);
        let rest: Vec<Name> = occs.iter().enumerate().filter(|(i, _)| *i != col).map(|(_, o)| o.clone()).collect();
        let defaults: Vec<Row> = rows.iter().filter(|r| wild(&r.pats[col])).map(|r| r.expand(col, vec![], &occ)).collect();
        let lits = rows.iter().any(|r| matches!(r.pats[col], Pat::Lit(_)));
        if lits && rows.iter().any(|r| matches!(r.pats[col], Pat::Ctor(..))) {
            return self.err_at(format!("{} の位置に構築子とリテラルのパターンは混ぜられない", shape));
        }
        if !lits {
            let mut heads: Vec<(Name, usize)> = Vec::new();
            for r in &rows {
                if let Pat::Ctor(c, xs) = &r.pats[col] {
                    match heads.iter().find(|h| h.0 == *c) {
                        Some(h) if h.1 != xs.len() => return self.err_at(format!("構築子 {} のフィールドの数が腕によって違う", ctor_label(c))),
                        Some(_) => {}
                        None => heads.push((c.clone(), xs.len())),
                    }
                }
            }
            let mut arms = Vec::new();
            for (c, k) in heads {
                let fresh: Vec<Name> = (0..k)
                    .map(|i| {
                        let holes: Vec<&str> = (0..k).map(|j| if i == j { "□" } else { "_" }).collect();
                        new_var(&shape.replacen('□', &format!("{}({})", ctor_label(&c), holes.join(", ")), 1))
                    })
                    .collect();
                let mut sub = Vec::new();
                for r in &rows {
                    match &r.pats[col] {
                        Pat::Ctor(c2, xs) if *c2 == c => sub.push(r.expand(col, xs.clone(), &occ)),
                        p if wild(p) => sub.push(r.expand(col, vec![Pat::Wild; k], &occ)),
                        _ => {}
                    }
                }
                let mut next = occs[..col].to_vec();
                next.extend(fresh.iter().cloned());
                next.extend(occs[col + 1..].iter().cloned());
                arms.push(Arm { span: None, ctor: c, binds: fresh, body: self.decide(next, sub)? });
            }
            if !defaults.is_empty() {
                arms.push(Arm { span: None, ctor: "_".into(), binds: vec![], body: self.decide(rest, defaults)? });
            }
            return Ok(Expr::Match(Box::new(Expr::Var(occ)), arms));
        }
        let mut values: Vec<Expr> = Vec::new();
        for r in &rows {
            if let Pat::Lit(v) = &r.pats[col]
                && !values.iter().any(|x| format!("{:?}", x.peel()) == format!("{:?}", v.peel()))
            {
                values.push(v.clone());
            }
        }
        let mut branches = Vec::new();
        for v in &values {
            let key = format!("{:?}", v.peel());
            let sub: Vec<Row> = rows
                .iter()
                .filter(|r| match &r.pats[col] {
                    Pat::Lit(x) => format!("{:?}", x.peel()) == key,
                    p => wild(p),
                })
                .map(|r| r.expand(col, vec![], &occ))
                .collect();
            branches.push((v.clone(), self.decide(rest.clone(), sub)?));
        }
        let both = values.len() == 2 && values.iter().all(|v| matches!(v.peel(), Expr::Bool(_)));
        let mut acc = if !defaults.is_empty() {
            self.decide(rest, defaults)?
        } else if both {
            branches.pop().unwrap().1
        } else {
            return self.err_at(format!("match に {} の場合が無い（文字列・数のパターンには _ の腕が要る）", shape.replacen('□', "_", 1)));
        };
        for (v, body) in branches.into_iter().rev() {
            let cond = Expr::Binary("==", Box::new(Expr::Var(occ.clone())), Box::new(v));
            acc = Expr::If(Box::new(cond), leaf(body), Some(Box::new(Expr::Block(leaf(acc)))));
        }
        Ok(acc)
    }

    // let パターン = 値。構造体の分解だけを受け付け、フィールドごとの let にする
    pub(super) fn destructure(&self, p: Pat, occ: Name, out: &mut Vec<Stmt>) -> R<()> {
        match p {
            Pat::Wild => Ok(()),
            Pat::Bind(x) => {
                out.push(Stmt::Let { name: x, mutable: false, ty: None, init: Expr::Var(occ) });
                Ok(())
            }
            Pat::Ctor(c, subs) => {
                let fields = c.structure_owner().and_then(|o| self.structs.get(&o)).map(|s| s.fields.clone());
                let Some(fields) = fields else {
                    return self.err_at(format!("let のパターンには構造体の分解だけを書ける（{} は構造体でない。網羅しないパターンは match を使う）", ctor_label(&c)));
                };
                if fields.len() != subs.len() {
                    return self.err_at(format!("構造体 {} のフィールドの数が違う", ctor_label(&c)));
                }
                for (f, sub) in fields.into_iter().zip(subs) {
                    let init = Expr::Field(Box::new(Expr::Var(occ.clone())), f);
                    match sub {
                        Pat::Wild => {}
                        Pat::Bind(x) => out.push(Stmt::Let { name: x, mutable: false, ty: None, init }),
                        sub => {
                            let t = new_var("□");
                            out.push(Stmt::Let { name: t.clone(), mutable: false, ty: None, init });
                            self.destructure(sub, t, out)?;
                        }
                    }
                }
                Ok(())
            }
            Pat::Lit(_) => self.err_at("let のパターンには構造体の分解だけを書ける（網羅しないパターンは match を使う）".into()),
        }
    }
}
