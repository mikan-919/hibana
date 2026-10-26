// syntax / macro 宣言の解析、パターン照合、マクロ展開
use crate::error::{Ctx, Error};
use crate::eval::{self, Value};
use crate::lexer::{Span, Tok, Token, lex};
use crate::parser::{Block, Expr, Parser, R, Stmt};
use crate::typeck;
use std::cell::OnceCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

// パターン照合の結果。検査用に全フィールドを保持する
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Node {
    Token(String, Span),
    Ident(String, Span),
    Int(i64),
    Float(f64),
    Str(String),
    Char(char),
    Text(String, Span),
    Expr(Expr),
    Block(Block),
    Stmt(Stmt),
    Type(Expr),
    // syntax 規則・マクロ・名前付きグループの束縛。rule は規則名
    Record {
        rule: Option<String>,
        span: Span,
        fields: Vec<(String, Node)>,
    },
    List(Vec<Node>),
    Null,
    // quote で作った、構造を持たないトークン列
    Raw,
    // 束縛した値と、それが占めるトークンの範囲（展開時にこのトークン列を差し込む）
    At(Range<usize>, Box<Node>),
}

impl Node {
    pub fn strip(&self) -> &Node {
        match self {
            Node::At(_, n) => n.strip(),
            n => n,
        }
    }
    pub fn get(&self, k: &str) -> Option<&Node> {
        match self.strip() {
            Node::Record { fields, .. } => fields.iter().find(|(n, _)| n == k).map(|(_, v)| v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Builtin {
    Ident,
    Int,
    Float,
    Str,
    Char,
    Expr,
    Block,
    Stmt,
    Type,
}

fn builtin(name: &str) -> Option<Builtin> {
    use Builtin::*;
    Some(match name {
        "Ident" => Ident,
        "Int" => Int,
        "Float" => Float,
        "String" => Str,
        "Char" => Char,
        "Expr" => Expr,
        "Block" => Block,
        "Stmt" => Stmt,
        "Type" => Type,
        _ => return None,
    })
}

// 選択枝の並び
#[derive(Debug, Clone)]
struct Pattern(Vec<Seq>);

#[derive(Debug, Clone)]
struct Seq(Vec<Term>);

#[derive(Debug, Clone)]
struct Term {
    adjacent: bool,
    name: Option<String>,
    atom: Atom,
    op: Op,
}

#[derive(Debug, Clone)]
enum Op {
    One,
    Opt,
    Many,
    Many1,
    Sep(Box<Atom>),
}

#[derive(Debug, Clone)]
enum Atom {
    Lit(String),
    Builtin(Builtin),
    Rule(String),
    Group(Pattern),
    // 停止集合（このリテラルで始まるトークンの手前で止まる）
    Text(Vec<String>),
}

// FIRST 集合の要素
#[derive(Debug, Clone, PartialEq)]
enum First {
    Lit(String),
    Class(Builtin),
    // Expr か Stmt。マクロの先頭にあると左再帰になる
    Expr,
    Any,
}

type Firsts = (Vec<First>, bool);

struct Rule {
    pat: Pattern,
    fields: Vec<(String, bool)>,
}

struct Macro {
    name: String,
    pat: Pattern,
    fields: Vec<(String, bool)>,
    // -> Stmt なら文の位置、-> Expr なら式の位置で試す
    stmt: bool,
    // 本体のトークン列（コンパイル時に実行するコード）。空なら照合結果を Expr::Macro として残す
    body: Option<Vec<Token>>,
    compiled: OnceCell<Block>,
    first: Firsts,
}

#[derive(Default)]
pub struct Grammar {
    rules: HashMap<String, Rule>,
    macros: Vec<Macro>,
}

// quote { ... } の要素
#[derive(Debug, Clone)]
pub enum Q {
    Tok(Token),
    // $a.b.c。rest は a の後ろの ". 名前" のトークン列
    Var {
        dollar: Token,
        path: Vec<String>,
        rest: Vec<Token>,
    },
    // ${ 式 }
    Splice(Token, Expr),
    // $( ... ) 区切り? *
    Rep(Vec<Q>, Option<Token>),
}
// パターンが持つ束縛名と、それが繰り返し（配列）かを求める。同じ連接内の重複はエラー
fn fields(p: &Pattern) -> R<Vec<(String, bool)>> {
    let mut all: Vec<(String, bool)> = Vec::new();
    for seq in &p.0 {
        let mut seen: Vec<(String, bool)> = Vec::new();
        for t in &seq.0 {
            let fs = match (&t.name, &t.atom) {
                (Some(n), _) => vec![(n.clone(), false)],
                // 無名グループの束縛は親に持ち上げる
                (None, Atom::Group(inner)) => {
                    let rep = !matches!(t.op, Op::One | Op::Opt);
                    fields(inner)?
                        .into_iter()
                        .map(|(n, r)| (n, r || rep))
                        .collect()
                }
                _ => vec![],
            };
            for f in fs {
                if seen.iter().any(|s| s.0 == f.0) {
                    return Err(format!("束縛名 {} が重複している", f.0).into());
                }
                seen.push(f);
            }
        }
        // 選択枝をまたぐ同名は1個のフィールドにまとめる
        for f in seen {
            if !all.iter().any(|a| a.0 == f.0) {
                all.push(f);
            }
        }
    }
    Ok(all)
}

// 全フィールドを Null か空配列で埋めてから束縛を書き込む
fn record(rule: Option<&str>, span: Span, fields: &[(String, bool)], binds: Binds) -> Node {
    let mut fs: Vec<(String, Node)> = fields
        .iter()
        .map(|(n, rep)| {
            (
                n.clone(),
                if *rep { Node::List(vec![]) } else { Node::Null },
            )
        })
        .collect();
    for (n, v) in binds {
        if let Some(f) = fs.iter_mut().find(|f| f.0 == n) {
            f.1 = v;
        }
    }
    Node::Record {
        rule: rule.map(String::from),
        span,
        fields: fs,
    }
}

fn collect_refs(p: &Pattern, out: &mut Vec<String>) {
    for t in p.0.iter().flat_map(|s| &s.0) {
        let sep = match &t.op {
            Op::Sep(a) => Some(&**a),
            _ => None,
        };
        for a in std::iter::once(&t.atom).chain(sep) {
            match a {
                Atom::Rule(n) => out.push(n.clone()),
                Atom::Group(inner) => collect_refs(inner, out),
                _ => {}
            }
        }
    }
}

fn lits(f: &[First]) -> Vec<String> {
    f.iter()
        .filter_map(|x| match x {
            First::Lit(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

// syn 型の値。node の At の範囲は toks を指す
#[derive(Debug)]
pub struct Syn {
    pub node: Node,
    pub toks: Rc<Vec<Token>>,
}

impl Syn {
    pub fn child(&self, node: Node) -> Syn {
        Syn {
            node,
            toks: self.toks.clone(),
        }
    }
}

// 型検査で syn の == を禁止しているので呼ばれない
impl PartialEq for Syn {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

// path を env の最長一致から辿り、(値, 辿った段数) を返す。stop_at_list なら配列で止まる
fn walk(env: &[(String, Value)], path: &[String], stop_at_list: bool) -> R<(Value, usize)> {
    let mut best: Option<(&Value, usize)> = None;
    for (k, v) in env {
        let ks: Vec<&str> = k.split('.').collect();
        let hit = ks.len() <= path.len() && ks.iter().zip(path).all(|(a, b)| a == b);
        // 同じ長さなら後に積んだ（内側の）束縛を優先する
        if hit && best.is_none_or(|b| ks.len() >= b.1) {
            best = Some((v, ks.len()));
        }
    }
    let (v, mut n) = best.ok_or(format!("未定義の変数 {}", path[0]))?;
    let mut v = v.clone();
    while n < path.len() {
        let Value::Syn(s) = &v else { break };
        if stop_at_list && matches!(s.node.strip(), Node::List(_)) {
            break;
        }
        let Some(c) = s.node.get(&path[n]) else { break };
        v = Value::Syn(Rc::new(s.child(c.clone())));
        n += 1;
    }
    Ok((v, n))
}

fn q_paths<'a>(q: &'a [Q], out: &mut Vec<&'a [String]>) {
    for x in q {
        match x {
            Q::Var { path, .. } => out.push(path),
            Q::Rep(inner, _) => q_paths(inner, out),
            Q::Tok(_) | Q::Splice(..) => {}
        }
    }
}

type Eval<'a> = dyn FnMut(&Expr, &[(String, Value)]) -> R<Value> + 'a;

// quote の中身を、env の値を差し込みながらトークン列にする。${ } は ev で評価する
pub fn splice(
    q: &[Q],
    env: &mut Vec<(String, Value)>,
    out: &mut Vec<Token>,
    ev: &mut Eval,
) -> R<()> {
    for x in q {
        match x {
            Q::Tok(t) => out.push(t.clone()),
            Q::Var { dollar, path, rest } => {
                let (v, n) = walk(env, path, false)?;
                emit(&v, dollar, out).map_err(|e| {
                    Error::from(format!("${}: {e}", path.join("."))).at(dollar.span)
                })?;
                // 束縛の中に無い残りの経路は、式のフィールド参照として残す
                out.extend(rest[2 * (n - 1)..].iter().cloned());
            }
            Q::Splice(dollar, e) => {
                let v = ev(e, env)?;
                emit(&v, dollar, out)
                    .map_err(|e| Error::from(format!("${{ }}: {e}")).at(dollar.span))?;
            }
            Q::Rep(inner, sep) => {
                let mut paths = Vec::new();
                q_paths(inner, &mut paths);
                let mut lists: Vec<(String, Rc<Syn>)> = Vec::new();
                for p in paths {
                    let (v, n) = walk(env, p, true)?;
                    let key = p[..n].join(".");
                    if let Value::Syn(s) = v
                        && matches!(s.node.strip(), Node::List(_))
                        && !lists.iter().any(|l| l.0 == key)
                    {
                        lists.push((key, s));
                    }
                }
                let len = |s: &Syn| match s.node.strip() {
                    Node::List(v) => v.len(),
                    _ => 0,
                };
                let Some(n) = lists.first().map(|l| len(&l.1)) else {
                    return Err("$( )* の中に配列の束縛が無い".into());
                };
                if lists.iter().any(|l| len(&l.1) != n) {
                    return Err("$( )* の中の配列の長さが揃っていない".into());
                }
                for k in 0..n {
                    if k > 0
                        && let Some(s) = sep
                    {
                        out.push(s.clone());
                    }
                    let base = env.len();
                    for (key, s) in &lists {
                        let Node::List(v) = s.node.strip() else {
                            unreachable!()
                        };
                        env.push((key.clone(), Value::Syn(Rc::new(s.child(v[k].clone())))));
                    }
                    let r = splice(inner, env, out, ev);
                    env.truncate(base);
                    r?;
                }
            }
        }
    }
    Ok(())
}

// 値をトークン列にして差し込む。先頭の空白・改行の有無は $ の位置に合わせる
fn emit(v: &Value, dollar: &Token, out: &mut Vec<Token>) -> R<()> {
    let at_dollar = |t: &Token| Token {
        gap_before: dollar.gap_before,
        nl_before: dollar.nl_before,
        ..t.clone()
    };
    let tok = |tok: Tok| Token {
        tok,
        gap_before: false,
        nl_before: false,
        ..dollar.clone()
    };
    let s = match v {
        Value::Int(n) if *n < 0 => {
            out.push(at_dollar(&tok(Tok::Punct("("))));
            out.extend([
                tok(Tok::Punct("-")),
                tok(Tok::Int(n.unsigned_abs() as i64)),
                tok(Tok::Punct(")")),
            ]);
            return Ok(());
        }
        Value::Int(n) => Tok::Int(*n),
        Value::Bool(b) => Tok::Punct(if *b { "true" } else { "false" }),
        Value::Str(s) => Tok::Str(s.to_string()),
        Value::Ty(t) => Tok::Ident(t.to_string()),
        Value::Arr(_) | Value::Data(_) | Value::Fn(_) => {
            return Err("配列・data・関数の値は差し込めない".into());
        }
        Value::Unit => return Err("() は差し込めない".into()),
        Value::Syn(s) => return emit_syn(s, dollar, out),
    };
    out.push(at_dollar(&tok(s)));
    Ok(())
}

fn emit_syn(s: &Syn, dollar: &Token, out: &mut Vec<Token>) -> R<()> {
    let at_dollar = |t: &Token| Token {
        gap_before: dollar.gap_before,
        nl_before: dollar.nl_before,
        ..t.clone()
    };
    let Node::At(range, inner) = &s.node else {
        return match s.node {
            Node::Null => Ok(()),
            _ => Err("配列は $( )* の中で使う".into()),
        };
    };
    let toks = &s.toks[range.clone()];
    match inner.strip() {
        Node::Null => Ok(()),
        Node::List(_) => Err("配列は $( )* の中で使う".into()),
        Node::Text(t, _) => {
            out.push(at_dollar(&Token {
                tok: Tok::Str(t.clone()),
                ..toks[0].clone()
            }));
            Ok(())
        }
        n => {
            let Some(first) = toks.first() else {
                return Ok(());
            };
            // 式は括弧で囲み、差し込み先の演算子と結合しないようにする
            // ponytail: quote で作った断片（Raw）は囲まない。必要なら quote の中で括弧を書く
            if matches!(n, Node::Expr(_)) {
                out.push(at_dollar(&Token {
                    tok: Tok::Punct("("),
                    ..first.clone()
                }));
                out.push(Token {
                    gap_before: false,
                    nl_before: false,
                    ..first.clone()
                });
                out.extend(toks[1..].iter().cloned());
                out.push(Token {
                    tok: Tok::Punct(")"),
                    gap_before: false,
                    nl_before: false,
                    ..first.clone()
                });
            } else {
                out.push(at_dollar(first));
                out.extend(toks[1..].iter().cloned());
            }
            Ok(())
        }
    }
}

impl Grammar {
    pub fn validate(&mut self) -> R<()> {
        let mut refs = Vec::new();
        for r in self.rules.values() {
            collect_refs(&r.pat, &mut refs);
        }
        for m in &self.macros {
            collect_refs(&m.pat, &mut refs);
        }
        if let Some(n) = refs.iter().find(|n| !self.rules.contains_key(*n)) {
            return Err(format!("未定義の規則 {n}").into());
        }
        for (n, r) in &self.rules {
            self.first_pat(&r.pat, &mut vec![n.clone()])?;
        }
        let names: Vec<String> = self.rules.keys().cloned().collect();
        for n in names {
            let mut p = self.rules[&n].pat.clone();
            self.resolve_text(&mut p, &[])?;
            self.rules.get_mut(&n).unwrap().pat = p;
        }
        for i in 0..self.macros.len() {
            let mut p = self.macros[i].pat.clone();
            self.resolve_text(&mut p, &[])?;
            let first = self.first_pat(&p, &mut vec![])?;
            if first.0.contains(&First::Expr) {
                return Err(format!(
                    "マクロ {} が Expr か Stmt で始まる（左再帰になる）",
                    self.macros[i].name
                )
                .into());
            }
            let m = &mut self.macros[i];
            m.pat = p;
            m.first = first;
        }
        Ok(())
    }

    // st は辿っている規則の列。同じ規則に先頭で戻ったら左再帰
    fn first_pat(&self, p: &Pattern, st: &mut Vec<String>) -> R<Firsts> {
        let (mut acc, mut nullable) = (Vec::new(), false);
        for s in &p.0 {
            let (f, n) = self.first_seq(&s.0, st)?;
            acc.extend(f);
            nullable |= n;
        }
        Ok((acc, nullable))
    }

    fn first_seq(&self, terms: &[Term], st: &mut Vec<String>) -> R<Firsts> {
        let mut acc = Vec::new();
        for t in terms {
            let (f, n) = self.first_term(t, st)?;
            acc.extend(f);
            if !n {
                return Ok((acc, false));
            }
        }
        Ok((acc, true))
    }

    fn first_term(&self, t: &Term, st: &mut Vec<String>) -> R<Firsts> {
        let (f, n) = match &t.atom {
            Atom::Lit(s) => (vec![First::Lit(s.clone())], false),
            Atom::Text(_) => (vec![First::Any], false),
            Atom::Builtin(b) => (
                vec![match b {
                    Builtin::Expr | Builtin::Stmt => First::Expr,
                    Builtin::Block => First::Lit("{".into()),
                    Builtin::Type => First::Class(Builtin::Ident),
                    b => First::Class(*b),
                }],
                false,
            ),
            Atom::Group(p) => self.first_pat(p, st)?,
            Atom::Rule(n) => {
                if st.contains(n) {
                    return Err(format!("規則 {n} が左再帰している").into());
                }
                st.push(n.clone());
                let r = self.first_pat(&self.rules[n].pat, st);
                st.pop();
                r?
            }
        };
        Ok((f, n || matches!(t.op, Op::Opt | Op::Many | Op::Sep(_))))
    }

    // Text の停止集合を、同じ選択の他の枝の先頭と、連接で次に来る項の先頭から決める
    fn resolve_text(&self, p: &mut Pattern, outer: &[String]) -> R<()> {
        let firsts: Vec<Vec<String>> =
            p.0.iter()
                .map(|s| Ok(lits(&self.first_seq(&s.0, &mut vec![])?.0)))
                .collect::<R<_>>()?;
        for i in 0..p.0.len() {
            let mut stop = outer.to_vec();
            for (j, f) in firsts.iter().enumerate() {
                if j != i {
                    stop.extend(f.iter().cloned());
                }
            }
            for k in 0..p.0[i].0.len() {
                let mut s = stop.clone();
                if let Some(next) = p.0[i].0.get(k + 1) {
                    s.extend(lits(&self.first_term(next, &mut vec![])?.0));
                }
                let t = &mut p.0[i].0[k];
                let sep = match &mut t.op {
                    Op::Sep(a) => Some(&mut **a),
                    _ => None,
                };
                for a in std::iter::once(&mut t.atom).chain(sep) {
                    match a {
                        Atom::Text(st) => *st = s.clone(),
                        Atom::Group(inner) => self.resolve_text(inner, &s)?,
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
}

// 照合失敗。Fail は巻き戻して次を試す。Fatal は解析全体を止める
enum Miss {
    Fail,
    Fatal(String),
}

type M<T> = Result<T, Miss>;
type Binds = Vec<(String, Node)>;

impl Parser<'_> {
    pub fn syntax_decl(&mut self, g: &mut Grammar) -> R<()> {
        self.expect("syntax")?;
        let name = self.ident()?;
        if builtin(&name).is_some() || name == "Text" || g.rules.contains_key(&name) {
            return self.err_at(format!("規則 {name} が重複している"));
        }
        self.expect("=")?;
        let pat = self.pattern()?;
        // 宣言の終わりは次の宣言の始まりで分かるので、";" は省ける
        self.eat(";");
        let fields = fields(&pat).ctx(|e| format!("規則 {name}: {e}"))?;
        g.rules.insert(name, Rule { pat, fields });
        Ok(())
    }

    pub fn macro_decl(&mut self, g: &mut Grammar) -> R<()> {
        self.expect("macro")?;
        let mut name = self.ident()?;
        if !self.toks[self.pos].gap_before && self.eat("!") {
            name.push('!');
        }
        if g.macros.iter().any(|m| m.name == name) {
            return self.err_at(format!("マクロ {name} が重複している"));
        }
        self.expect("(")?;
        let pat = self.pattern()?;
        self.expect(")")?;
        let stmt = if self.eat("->") {
            match self.ident()?.as_str() {
                "Expr" => false,
                "Stmt" => true,
                _ => return self.err_at("マクロの戻り値は Expr か Stmt".into()),
            }
        } else {
            false
        };
        self.expect("{")?;
        let start = self.pos;
        let mut depth = 1;
        while depth > 0 {
            match self.next() {
                Tok::Punct("{") => depth += 1,
                Tok::Punct("}") => depth -= 1,
                Tok::Eof => return self.err("マクロ本体の \"}\" が足りない"),
                _ => {}
            }
        }
        let fields = fields(&pat).ctx(|e| format!("マクロ {name}: {e}"))?;
        let raw = &self.toks[start..self.pos - 1];
        let body = if raw.is_empty() {
            None
        } else {
            Some(raw.to_vec())
        };
        g.macros.push(Macro {
            name,
            pat,
            fields,
            stmt,
            body,
            first: (vec![], true),
            compiled: OnceCell::new(),
        });
        Ok(())
    }

    fn pattern(&mut self) -> R<Pattern> {
        self.eat("|");
        let mut branches = vec![self.seq()?];
        while self.eat("|") {
            branches.push(self.seq()?);
        }
        Ok(Pattern(branches))
    }

    fn seq(&mut self) -> R<Seq> {
        let mut terms = Vec::new();
        while matches!(
            self.peek(),
            Tok::Punct("~" | "(") | Tok::Ident(_) | Tok::Str(_)
        ) {
            terms.push(self.term()?);
        }
        if terms.is_empty() {
            return self.err("パターンの項が必要");
        }
        Ok(Seq(terms))
    }

    fn term(&mut self) -> R<Term> {
        let adjacent = self.eat("~");
        let mut name = None;
        if let Tok::Ident(n) = self.peek().clone()
            && matches!(
                self.toks.get(self.pos + 1).map(|t| &t.tok),
                Some(Tok::Punct(":"))
            )
        {
            self.pos += 2;
            name = Some(n);
        }
        let atom = self.atom()?;
        let op = if self.eat("*") {
            Op::Many
        } else if self.eat("+") {
            Op::Many1
        } else if self.eat("?") {
            Op::Opt
        } else if self.eat("%") {
            Op::Sep(Box::new(self.atom()?))
        } else {
            Op::One
        };
        Ok(Term {
            adjacent,
            name,
            atom,
            op,
        })
    }

    fn atom(&mut self) -> R<Atom> {
        let a = match self.peek().clone() {
            Tok::Str(s) => {
                let ts = lex(&s)?;
                if ts.len() != 2 || ts[0].tok.text() != Some(s.as_str()) {
                    return self.err_at(format!("リテラル {s:?} は1個のトークンでない"));
                }
                Atom::Lit(s)
            }
            Tok::Ident(n) if n == "Text" => Atom::Text(vec![]),
            Tok::Ident(n) => builtin(&n).map(Atom::Builtin).unwrap_or(Atom::Rule(n)),
            Tok::Punct("(") => {
                self.pos += 1;
                let p = self.pattern()?;
                self.expect(")")?;
                return Ok(Atom::Group(p));
            }
            _ => return self.err("パターンの項が必要"),
        };
        self.pos += 1;
        Ok(a)
    }

    // 文の位置（stmt）か式の位置で、全マクロを照合する
    pub fn try_macros(&mut self, stmt: bool) -> R<Option<Expr>> {
        let g = self.g.clone();
        let start = self.pos;
        let mut hits = Vec::new();
        for m in g.macros.iter().filter(|m| m.stmt == stmt) {
            self.pos = start;
            if !self.could_start(&m.first) {
                continue;
            }
            let mut b = Vec::new();
            match self.m_pattern(&m.pat, &mut b) {
                Ok(_) => hits.push((m, b, self.pos)),
                Err(Miss::Fail) => {}
                Err(Miss::Fatal(e)) => return self.err_at(e),
            }
        }
        self.pos = start;
        match hits.len() {
            0 => Ok(None),
            1 => {
                let (m, b, end) = hits.pop().unwrap();
                self.pos = end;
                let rec = record(Some(&m.name), self.span_from(start), &m.fields, b);
                match &m.body {
                    None => Ok(Some(Expr::Macro(m.name.clone(), Box::new(rec)))),
                    Some(_) => self.expand(m, rec).map(Some),
                }
            }
            _ => {
                let names: Vec<&str> = hits.iter().map(|h| h.0.name.as_str()).collect();
                self.err_at(format!("Ambiguous macro match（{}）", names.join(", ")))
            }
        }
    }

    // マクロ本体を構文解析・型検査する。結果は使い回す
    fn macro_block(&self, m: &Macro) -> R<Block> {
        if let Some(b) = m.compiled.get() {
            return Ok(b.clone());
        }
        let raw = m.body.as_ref().unwrap();
        let end = raw.last().unwrap().clone();
        let mut toks = raw.clone();
        toks.extend([
            Token {
                tok: Tok::Punct("}"),
                nl_before: true,
                ..end.clone()
            },
            Token {
                tok: Tok::Eof,
                ..end
            },
        ]);
        let mut p = Parser {
            toks,
            pos: 0,
            src: self.src,
            g: self.g.clone(),
            depth: self.depth + 1,
            types: self.types.clone(),
            generic_fns: self.generic_fns.clone(),
        };
        let wrap = |e: &str| format!("マクロ {}: {e}", m.name);
        let b = p.block().ctx(wrap)?;
        let names: Vec<String> = m.fields.iter().map(|f| f.0.clone()).collect();
        typeck::check_macro(&b, &names).ctx(wrap)?;
        let _ = m.compiled.set(b.clone());
        Ok(b)
    }

    // 全マクロの本体を先に検査し、使われていないマクロの誤りも報告する
    pub fn check_macros(&self) -> R<()> {
        for m in self.g.macros.iter().filter(|m| m.body.is_some()) {
            self.macro_block(m)?;
        }
        Ok(())
    }

    // ponytail: 衛生的でない。展開コード内の let は呼び出し側の変数と衝突しうる
    // ponytail: 展開のたびにトークン列全体を複製する。遅ければ Parser.toks を Rc にする
    fn expand(&self, m: &Macro, rec: Node) -> R<Expr> {
        if self.depth >= 64 {
            return Err(format!("マクロ {} の展開が深すぎる", m.name).into());
        }
        let block = self.macro_block(m)?;
        let Node::Record { fields, .. } = rec else {
            unreachable!()
        };
        let src_toks = Rc::new(self.toks.clone());
        let vars = fields
            .into_iter()
            .map(|(n, node)| {
                (
                    n,
                    Value::Syn(Rc::new(Syn {
                        node,
                        toks: src_toks.clone(),
                    })),
                )
            })
            .collect();
        let wrap = |e: &str| format!("マクロ {} の展開: {e}", m.name);
        let v = eval::run_macro(&block, vars)
            .map_err(Error::from)
            .ctx(wrap)?;
        let end = self.toks[self.pos].clone();
        let mut toks = Vec::new();
        emit(&v, &end, &mut toks).ctx(wrap)?;
        let eof = Token {
            tok: Tok::Eof,
            ..end.clone()
        };
        let mut p = Parser {
            toks,
            pos: 0,
            src: self.src,
            g: self.g.clone(),
            depth: self.depth + 1,
            types: self.types.clone(),
            generic_fns: self.generic_fns.clone(),
        };
        if m.stmt {
            // 文の並びとして読み、ブロックにする
            p.toks.extend([
                Token {
                    tok: Tok::Punct("}"),
                    nl_before: true,
                    ..end
                },
                eof,
            ]);
            return p.block().map(Expr::Block).ctx(wrap);
        }
        p.toks.push(eof);
        let e = p.expr().ctx(wrap)?;
        if p.peek() != &Tok::Eof {
            return p.err("1個の式でない").ctx(wrap);
        }
        Ok(e)
    }

    fn could_start(&self, (f, nullable): &Firsts) -> bool {
        let t = &self.toks[self.pos].tok;
        *nullable
            || f.iter().any(|x| match x {
                First::Lit(s) => t.text() == Some(s.as_str()),
                First::Class(b) => matches!(
                    (b, t),
                    (Builtin::Ident, Tok::Ident(_))
                        | (Builtin::Int, Tok::Int(_))
                        | (Builtin::Float, Tok::Float(_))
                        | (Builtin::Str, Tok::Str(_))
                        | (Builtin::Char, Tok::Char(_))
                ),
                First::Expr | First::Any => true,
            })
    }

    pub fn span_from(&self, start: usize) -> Span {
        let s = self.toks[start].span.start;
        let end = if self.pos > start {
            self.toks[self.pos - 1].span.end
        } else {
            s
        };
        Span { start: s, end }
    }

    // 選択枝を順に試し、最初に一致した枝を採る
    fn m_pattern(&mut self, p: &Pattern, b: &mut Binds) -> M<Node> {
        let save = self.pos;
        for seq in &p.0 {
            let mut sb = Vec::new();
            match self.m_seq(seq, &mut sb) {
                Ok(v) => {
                    b.extend(sb);
                    return Ok(v);
                }
                Err(Miss::Fail) => self.pos = save,
                Err(e) => return Err(e),
            }
        }
        Err(Miss::Fail)
    }

    fn m_seq(&mut self, seq: &Seq, b: &mut Binds) -> M<Node> {
        let mut vals = Vec::new();
        for t in &seq.0 {
            if t.adjacent && self.toks[self.pos].gap_before {
                return Err(Miss::Fail);
            }
            let start = self.pos;
            let v = match &t.name {
                Some(n) => {
                    let mut inner = Vec::new();
                    let v = self.m_term(t, &mut inner)?;
                    let v = if inner.is_empty() {
                        v
                    } else {
                        Node::Record {
                            rule: None,
                            span: self.span_from(start),
                            fields: inner,
                        }
                    };
                    let v = Node::At(start..self.pos, Box::new(v));
                    b.push((n.clone(), v.clone()));
                    v
                }
                None => self.m_term(t, b)?,
            };
            vals.push(v);
        }
        Ok(if vals.len() == 1 {
            vals.pop().unwrap()
        } else {
            Node::List(vals)
        })
    }

    // 繰り返しの1回分の束縛を、名前ごとの配列に積む
    fn push_binds(lists: &mut Vec<(String, Vec<Node>)>, ib: Binds) {
        for (n, v) in ib {
            match lists.iter_mut().find(|l| l.0 == n) {
                Some(l) => l.1.push(v),
                None => lists.push((n, vec![v])),
            }
        }
    }

    // 繰り返しの1回分を照合し、要素を範囲付きで積む
    fn m_item(
        &mut self,
        a: &Atom,
        items: &mut Vec<Node>,
        lists: &mut Vec<(String, Vec<Node>)>,
    ) -> M<()> {
        let save = self.pos;
        let mut ib = Vec::new();
        let v = self.m_atom(a, &mut ib)?;
        items.push(Node::At(save..self.pos, Box::new(v)));
        Self::push_binds(lists, ib);
        Ok(())
    }

    fn m_term(&mut self, t: &Term, b: &mut Binds) -> M<Node> {
        let mut items = Vec::new();
        let mut lists = Vec::new();
        match &t.op {
            Op::One => return self.m_atom(&t.atom, b),
            Op::Opt => {
                let save = self.pos;
                let mut ob = Vec::new();
                return match self.m_atom(&t.atom, &mut ob) {
                    Ok(v) => {
                        b.extend(ob);
                        Ok(v)
                    }
                    Err(Miss::Fail) => {
                        self.pos = save;
                        Ok(Node::Null)
                    }
                    Err(e) => Err(e),
                };
            }
            Op::Many | Op::Many1 => loop {
                let save = self.pos;
                match self.m_item(&t.atom, &mut items, &mut lists) {
                    // 空一致は無限ループになるので止める
                    Ok(()) if self.pos == save => {
                        items.pop();
                        break;
                    }
                    Ok(()) => {}
                    Err(Miss::Fail) => {
                        self.pos = save;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            },
            Op::Sep(sep) => {
                let save = self.pos;
                match self.m_item(&t.atom, &mut items, &mut lists) {
                    Ok(()) => loop {
                        let save = self.pos;
                        if let Err(e) = self.m_atom(sep, &mut Vec::new()) {
                            if let Miss::Fatal(_) = e {
                                return Err(e);
                            }
                            self.pos = save;
                            break;
                        }
                        match self.m_item(&t.atom, &mut items, &mut lists) {
                            Ok(()) => {}
                            Err(Miss::Fail) => {
                                return Err(Miss::Fatal("区切りの後に要素が無い".into()));
                            }
                            Err(e) => return Err(e),
                        }
                    },
                    Err(Miss::Fail) => self.pos = save,
                    Err(e) => return Err(e),
                }
            }
        }
        if matches!(t.op, Op::Many1) && items.is_empty() {
            return Err(Miss::Fail);
        }
        b.extend(lists.into_iter().map(|(n, v)| (n, Node::List(v))));
        Ok(Node::List(items))
    }

    fn m_atom(&mut self, a: &Atom, b: &mut Binds) -> M<Node> {
        let start = self.pos;
        match a {
            Atom::Lit(s) => {
                let t = &self.toks[self.pos];
                if t.tok.text() != Some(s.as_str()) {
                    return Err(Miss::Fail);
                }
                let span = t.span;
                self.pos += 1;
                Ok(Node::Token(s.clone(), span))
            }
            Atom::Builtin(k) => self.m_builtin(*k),
            Atom::Rule(name) => {
                let g = self.g.clone();
                let r = &g.rules[name];
                let mut rb = Vec::new();
                self.m_pattern(&r.pat, &mut rb)?;
                Ok(record(Some(name), self.span_from(start), &r.fields, rb))
            }
            Atom::Group(p) => self.m_pattern(p, b),
            Atom::Text(stop) => self.m_text(stop),
        }
    }

    fn m_builtin(&mut self, k: Builtin) -> M<Node> {
        let t = self.toks[self.pos].clone();
        let fail = |_| Miss::Fail;
        let n = match (k, t.tok) {
            (Builtin::Ident, Tok::Ident(s)) => Node::Ident(s, t.span),
            (Builtin::Int, Tok::Int(n)) => Node::Int(n),
            (Builtin::Float, Tok::Float(n)) => Node::Float(n),
            (Builtin::Str, Tok::Str(s)) => Node::Str(s),
            (Builtin::Char, Tok::Char(c)) => Node::Char(c),
            // ponytail: コア構文のエラーは Fail として巻き戻すため、入れ子のマクロの曖昧性エラーも消える
            (Builtin::Expr, _) => return self.expr().map(Node::Expr).map_err(fail),
            (Builtin::Stmt, _) => return self.stmt().map(Node::Stmt).map_err(fail),
            (Builtin::Type, _) => return self.ty().map(Node::Type).map_err(fail),
            (Builtin::Block, _) if self.eat("{") => {
                return self.block().map(Node::Block).map_err(fail);
            }
            _ => return Err(Miss::Fail),
        };
        self.pos += 1;
        Ok(n)
    }

    // ponytail: トークン単位で切り出すので、字句解析できない文字（例: Don't の '）は Text に書けない
    fn m_text(&mut self, stop: &[String]) -> M<Node> {
        let start = self.pos;
        loop {
            let stops = match self.peek() {
                Tok::Eof => true,
                Tok::Punct(p) => stop.iter().any(|s| p.starts_with(s.as_str())),
                Tok::Ident(i) => stop.iter().any(|s| s == i),
                _ => false,
            };
            if stops {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(Miss::Fail);
        }
        let raw =
            &self.src[self.toks[start].span.start.offset..self.toks[self.pos].span.start.offset];
        // JSX と同じく、改行を含む末尾の空白は捨てる
        let trimmed = raw.trim_end();
        let text = if raw[trimmed.len()..].contains('\n') {
            trimmed
        } else {
            raw
        };
        Ok(Node::Text(text.to_string(), self.span_from(start)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    // main の最初の let に書いたマクロの照合結果を取り出す
    fn macro_node(src: &str) -> Node {
        let fns = parse(src).unwrap().fns;
        let main = fns.iter().find(|f| f.name == "main").unwrap();
        match &main.body.stmts[0] {
            Stmt::Let { init, .. } if matches!(init.peel(), Expr::Macro(..)) => {
                let Expr::Macro(_, n) = init.peel() else {
                    unreachable!()
                };
                (**n).clone()
            }
            s => panic!("{s:?}"),
        }
    }

    fn f<'a>(n: &'a Node, k: &str) -> &'a Node {
        n.get(k)
            .unwrap_or_else(|| panic!("{k} が無い: {n:?}"))
            .strip()
    }

    fn list(n: &Node) -> &[Node] {
        match n.strip() {
            Node::List(v) => v,
            _ => panic!("{n:?}"),
        }
    }

    fn ident(n: &Node) -> &str {
        match n.strip() {
            Node::Ident(s, _) => s,
            _ => panic!("{n:?}"),
        }
    }

    const ROUTES: &str = r#"
syntax Annotation = "@" ~ name:Ident "(" args:ExprList? ")" "->" alias:Ident ;
syntax ExprList = items:(Expr % ",") ;
syntax Route = annotations:Annotation* method:Ident path:String body:Block ;
macro routes! ("routes" ~ "!" "{" rs:Route* "}") -> Expr {
    // 展開コード
}
fn main() {
    let r = routes! {
        GET "/user/{id}" {
            return Response.json({ id });
        }

        @durable_objects("ACCOUNT") -> account
        POST "/post/{user}/{post}" {
            let stub = account.get(user);
            return stub.fetch(request);
        }
    }
}
"#;

    #[test]
    fn routes() {
        let n = macro_node(ROUTES);
        let rs = list(f(&n, "rs"));
        assert_eq!(rs.len(), 2);
        assert_eq!(ident(f(&rs[0], "method")), "GET");
        assert!(list(f(&rs[0], "annotations")).is_empty());
        let a = &list(f(&rs[1], "annotations"))[0];
        assert_eq!(ident(f(a, "name")), "durable_objects");
        assert_eq!(ident(f(a, "alias")), "account");
        assert_eq!(list(f(f(a, "args"), "items")).len(), 1);
        assert!(matches!(f(&rs[1], "path"), Node::Str(s) if s == "/post/{user}/{post}"));

        assert!(parse(&ROUTES.replace("routes! {", "routes ! {")).is_err());
        assert!(parse(&ROUTES.replace("@durable", "@ durable")).is_err());
    }

    #[test]
    fn jsx() {
        let n = macro_node(
            r#"
syntax JsxAttr = name:Ident "=" "{" value:Expr "}"
syntax JsxElement =
    "<" ~ tag:Ident attrs:JsxAttr* ">"
    children:JsxChild*
    "</" ~ close:Ident ~ ">" ;
syntax JsxChild =
    | el:JsxElement
    | "{" expr:Expr "}"
    | text:Text
macro jsx (el:JsxElement) -> Expr {}
fn main() {
    let v = <Button onClick={handleClick}>
        Hello {userName}!
        <Icon name={"star"}></Icon>
    </Button>
}
"#,
        );
        let el = f(&n, "el");
        assert_eq!(ident(f(el, "tag")), "Button");
        assert_eq!(list(f(el, "attrs")).len(), 1);
        let cs = list(f(el, "children"));
        assert_eq!(cs.len(), 4);
        assert!(matches!(f(&cs[0], "text"), Node::Text(s, _) if s == "Hello "));
        assert!(
            matches!(f(&cs[1], "expr"), Node::Expr(e) if matches!(e.peel(), Expr::Var(s) if s == "userName"))
        );
        assert!(matches!(f(&cs[2], "text"), Node::Text(s, _) if s == "!"));
        assert_eq!(ident(f(f(&cs[3], "el"), "tag")), "Icon");
        assert!(matches!(f(&cs[3], "text"), Node::Null));
    }

    #[test]
    fn definition_errors() {
        for (src, msg) in [
            ("syntax L = items:Expr (\",\" items:Expr)* ;", "重複"),
            ("syntax E = E \"+\" Int ;", "左再帰"),
            ("syntax A = \"a b\" ;", "1個のトークンでない"),
            ("syntax A = B ;", "未定義の規則"),
            ("macro m (e:Expr \"!\") {}", "左再帰"),
            ("macro m (\"m\") { quote { $x } }", "未定義の変数 x"),
            ("macro m (\"m\") { 1 }", "マクロ本体の値"),
            (
                "macro m (\"m\") { error(\"だめ\") } fn main() { let x = m }",
                "だめ",
            ),
            (
                "macro m (\"m\" x:Int) { quote { $( $x ) } }",
                "\"*\" が必要",
            ),
            (
                "macro a (\"hey\") {} macro b (\"hey\") {} fn main() { let x = hey }",
                "Ambiguous",
            ),
            (
                "syntax L = (Expr % \",\") ; macro m (\"m\" L) {} fn main() { let x = m 1, }",
                "区切り",
            ),
            (
                "macro m (\"m\" x:Int*) { quote { $x } } fn main() { let y = m 1 2 }",
                "$( )* の中",
            ),
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.contains(msg), "{src} => {e}");
        }
    }
}
