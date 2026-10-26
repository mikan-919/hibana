// 型検査を通ったプログラムから型を消し、JavaScript を出力する
//
// 値の表し方: int は BigInt（64ビットを超えたら実行時エラー）、bool・str は JavaScript の値、
// unit は undefined、Vec は配列、type の値は { $c: 構築子名, $f: [フィールド] }、関数は関数、
// 型は名前の文字列。式は1つずつ一時変数に入れて、評価の順序をインタプリタと揃える
use crate::parser::{Arm, Block, Expr, Fn, Program, Stmt};
use std::collections::HashMap;

const PRELUDE: &str = r#""use strict";
const $MIN = -(2n ** 63n), $MAX = 2n ** 63n - 1n;
function $chk(x) { if (x < $MIN || x > $MAX) throw new Error("オーバーフロー"); return x; }
function $add(a, b) { return typeof a === "string" ? a + b : $chk(a + b); }
function $sub(a, b) { return $chk(a - b); }
function $mul(a, b) { return $chk(a * b); }
function $div(a, b) { if (b === 0n) throw new Error("0 で除算した"); return $chk(a / b); }
function $rem(a, b) { if (b === 0n) throw new Error("0 で除算した"); return a % b; }
function $neg(a) { return $chk(-a); }
function $d(c, f) { return { $c: c, $f: f }; }
function $field(x, f) { return x.$f[$fields[x.$c].indexOf(f)]; }
function $eq(a, b) {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => $eq(x, b[i]));
  if (a && b && typeof a === "object" && typeof b === "object") return a.$c === b.$c && $eq(a.$f, b.$f);
  return false;
}
function $show(v) {
  if (typeof v === "bigint" || typeof v === "boolean") return String(v);
  if (typeof v === "string") return JSON.stringify(v);
  if (v === undefined) return "()";
  if (typeof v === "function") return "<fn>";
  if (Array.isArray(v)) return "[" + v.map($show).join(", ") + "]";
  if (v.$c.endsWith("{}")) return v.$c.slice(0, -2) + " { " + $fields[v.$c].map((f, i) => f + ": " + $show(v.$f[i])).join(", ") + " }";
  return v.$f.length === 0 ? v.$c : v.$c + "(" + v.$f.map($show).join(", ") + ")";
}
function $str(v) { return typeof v === "string" ? v : $show(v); }
function $print(v) { console.log($str(v)); }
function $len(v) { return BigInt(typeof v === "string" ? [...v].length : v.length); }
function $split(s, sep) { return s.split(sep); }
function $starts_with(s, p) { return s.startsWith(p); }
function $slice(s, i, j) {
  const cs = [...s];
  if (!(0n <= i && i <= j && j <= BigInt(cs.length))) throw new Error(`slice の範囲 ${i}..${j} が文字列の外`);
  return cs.slice(Number(i), Number(j)).join("");
}
"#;

pub fn emit(prog: &Program) -> String {
    let mut g = Gen {
        fns: prog.fns.iter().map(|f| (f.name.as_str(), f)).collect(),
        ctors: prog
            .datas
            .iter()
            .flat_map(|d| &d.ctors)
            .map(|c| c.name.as_str())
            .collect(),
        lines: Vec::new(),
        scopes: Vec::new(),
        next: 0,
    };
    let mut out = String::from(PRELUDE);
    let fields: Vec<String> = prog
        .datas
        .iter()
        .flat_map(|d| &d.ctors)
        .map(|c| {
            let fs: Vec<String> = c.fields.iter().map(|f| lit(&f.0)).collect();
            format!("{}: [{}]", lit(&c.name), fs.join(", "))
        })
        .collect();
    out += &format!("const $fields = {{ {} }};\n", fields.join(", "));
    for f in &prog.fns {
        out += &g.func(f);
    }
    out += "try { f_main(); } catch (e) { console.error(\"実行時エラー: \" + e.message); process.exitCode = 1; }\n";
    out
}

// JavaScript の文字列リテラル
fn lit(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o += "\\\"",
            '\\' => o += "\\\\",
            '\n' => o += "\\n",
            '\r' => o += "\\r",
            '\t' => o += "\\t",
            '\0' => o += "\\0",
            c => o.push(c),
        }
    }
    o + "\""
}

// 衛生性で付けた # などを JavaScript の名前に使える形にする
fn ident(prefix: &str, n: &str) -> String {
    format!("{prefix}{}", n.replace('#', "$").replace('.', "$_"))
}

struct Gen<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    ctors: std::collections::HashSet<&'a str>,
    // 今の関数の文
    lines: Vec<String>,
    // hibana の変数名から (JavaScript の名前, 可変か)
    scopes: Vec<HashMap<String, (String, bool)>>,
    next: usize,
}

impl Gen<'_> {
    fn tmp(&mut self) -> String {
        self.next += 1;
        format!("t{}", self.next)
    }

    fn push(&mut self, s: String) {
        self.lines.push(s);
    }

    // 同じ名前の let が重なっても衝突しないよう、束縛ごとに番号を付ける
    fn bind(&mut self, n: &str, mutable: bool) -> String {
        self.next += 1;
        let js = format!("{}_{}", ident("v_", n), self.next);
        self.scopes
            .last_mut()
            .unwrap()
            .insert(n.to_string(), (js.clone(), mutable));
        js
    }

    fn lookup(&self, n: &str) -> Option<&(String, bool)> {
        self.scopes.iter().rev().find_map(|s| s.get(n))
    }

    // 関数の本体を、別の文の並びとして出力する
    fn function(&mut self, params: &[String], body: &Block) -> String {
        let saved = std::mem::take(&mut self.lines);
        self.scopes.push(HashMap::new());
        let ps: Vec<String> = params.iter().map(|p| self.bind(p, false)).collect();
        let r = self.block(body);
        self.push(format!("return {r};"));
        self.scopes.pop();
        let lines = std::mem::replace(&mut self.lines, saved);
        format!("function({}) {{\n{}\n}}", ps.join(", "), lines.join("\n"))
    }

    fn func(&mut self, f: &Fn) -> String {
        let params: Vec<String> = f.params.iter().map(|p| p.0.clone()).collect();
        format!(
            "const {} = {};\n",
            ident("f_", &f.name),
            self.function(&params, &f.body)
        )
    }

    fn block(&mut self, b: &Block) -> String {
        self.scopes.push(HashMap::new());
        for s in &b.stmts {
            match s {
                Stmt::Let {
                    name,
                    mutable,
                    init,
                    ..
                } => {
                    let v = self.expr(init);
                    let js = self.bind(name, *mutable);
                    self.push(format!("let {js} = {v};"));
                }
                Stmt::Expr(e) => {
                    self.expr(e);
                }
            }
        }
        let r = match &b.tail {
            Some(e) => self.expr(e),
            None => "undefined".into(),
        };
        self.scopes.pop();
        r
    }

    // 波括弧の中に文を出力し、値を t に入れる
    fn branch(&mut self, t: &str, f: impl FnOnce(&mut Self) -> String) {
        self.push("{".into());
        let v = f(self);
        self.push(format!("{t} = {v};"));
        self.push("}".into());
    }

    // 値を一時変数に入れて、その名前を返す
    fn let_(&mut self, v: String) -> String {
        let t = self.tmp();
        self.push(format!("const {t} = {v};"));
        t
    }

    fn args(&mut self, args: &[Expr]) -> Vec<String> {
        args.iter().map(|a| self.expr(a)).collect()
    }

    fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::At(_, x) => self.expr(x),
            Expr::Int(n) => format!("{n}n"),
            Expr::Bool(b) => b.to_string(),
            Expr::Str(s) => lit(s),
            Expr::Var(n) => match self.lookup(n) {
                Some((js, _)) => js.clone(),
                None if self.ctors.contains(n.as_str()) => self.let_(format!("$d({}, [])", lit(n))),
                None => match self.fns.get(n.as_str()) {
                    Some(f) if f.alias && f.params.is_empty() => {
                        self.let_(format!("{}()", ident("f_", n)))
                    }
                    Some(_) => ident("f_", n),
                    // 型は名前の文字列
                    None => lit(n),
                },
            },
            Expr::Unary(op, x) => {
                let a = self.expr(x);
                self.let_(if *op == "-" {
                    format!("$neg({a})")
                } else {
                    format!("!{a}")
                })
            }
            Expr::Binary(op @ ("&&" | "||"), l, r) => {
                let a = self.expr(l);
                let t = self.tmp();
                self.push(format!("let {t} = {a};"));
                self.push(format!("if ({}{t})", if *op == "&&" { "" } else { "!" }));
                self.branch(&t, |g| g.expr(r));
                t
            }
            Expr::Binary(op, l, r) => {
                let (a, b) = (self.expr(l), self.expr(r));
                let v = match *op {
                    "+" => format!("$add({a}, {b})"),
                    "-" => format!("$sub({a}, {b})"),
                    "*" => format!("$mul({a}, {b})"),
                    "/" => format!("$div({a}, {b})"),
                    "%" => format!("$rem({a}, {b})"),
                    "==" => format!("$eq({a}, {b})"),
                    "!=" => format!("!$eq({a}, {b})"),
                    op => format!("({a} {op} {b})"),
                };
                self.let_(v)
            }
            Expr::Call(name, args) => {
                if let Some((js, _)) = self.lookup(name).cloned() {
                    let vs = self.args(args);
                    return self.let_(format!("{js}({})", vs.join(", ")));
                }
                let mut vs = self.args(args);
                let v = match name.as_str() {
                    "print" => format!("$print({})", vs[0]),
                    "to_str" => format!("$str({})", vs[0]),
                    "len" | "split" | "starts_with" | "slice" => {
                        format!("${name}({})", vs.join(", "))
                    }
                    n if self.ctors.contains(n) => format!("$d({}, [{}])", lit(n), vs.join(", ")),
                    n => match self.fns.get(n) {
                        Some(f) => {
                            // 推論したジェネリック引数は実行時には要らないので undefined で埋める
                            if vs.len() + f.implicit == f.params.len() {
                                let mut full = vec!["undefined".to_string(); f.implicit];
                                full.append(&mut vs);
                                vs = full;
                            }
                            format!("{}({})", ident("f_", n), vs.join(", "))
                        }
                        // 型を作る呼び出し（Vec<int> など）。実行時には名前だけ
                        None => lit(n),
                    },
                };
                self.let_(v)
            }
            Expr::Apply(f, args) => {
                let f = self.expr(f);
                let vs = self.args(args);
                self.let_(format!("{f}({})", vs.join(", ")))
            }
            Expr::Field(x, f) => {
                let a = self.expr(x);
                self.let_(format!("$field({a}, {})", lit(f)))
            }
            Expr::Index(v, i) => {
                let (a, b) = (self.expr(v), self.expr(i));
                self.let_(format!("{a}[Number({b})]"))
            }
            Expr::Array(items) => {
                let vs = self.args(items);
                self.let_(format!("[{}]", vs.join(", ")))
            }
            Expr::Assign(n, x) => {
                let v = self.expr(x);
                let js = self.lookup(n).expect("型検査済み").0.clone();
                self.push(format!("{js} = {v};"));
                "undefined".into()
            }
            Expr::If(c, then, els) => {
                let c = self.expr(c);
                let t = self.tmp();
                self.push(format!("let {t};"));
                self.push(format!("if ({c})"));
                self.branch(&t, |g| g.block(then));
                if let Some(els) = els {
                    self.push("else".into());
                    self.branch(&t, |g| g.expr(els));
                }
                t
            }
            Expr::While(c, body) => {
                self.push("while (true) {".into());
                let c = self.expr(c);
                self.push(format!("if (!{c}) break;"));
                self.block(body);
                self.push("}".into());
                "undefined".into()
            }
            Expr::For(x, it, end, body) => {
                let a = self.expr(it);
                let head = match end {
                    Some(end) => {
                        let b = self.expr(end);
                        let i = self.tmp();
                        (format!("for (let {i} = {a}; {i} < {b}; {i}++) {{"), i)
                    }
                    None => {
                        let i = self.tmp();
                        (format!("for (const {i} of {a}) {{"), i)
                    }
                };
                self.push(head.0);
                self.scopes.push(HashMap::new());
                let js = self.bind(x, false);
                self.push(format!("const {js} = {};", head.1));
                self.block(body);
                self.scopes.pop();
                self.push("}".into());
                "undefined".into()
            }
            Expr::Match(s, arms) => self.match_(s, arms),
            Expr::Block(b) => {
                let t = self.tmp();
                self.push(format!("let {t};"));
                self.branch(&t, |g| g.block(b));
                t
            }
            Expr::Return(x) => {
                let v = match x {
                    Some(x) => self.expr(x),
                    None => "undefined".into(),
                };
                self.push(format!("return {v};"));
                "undefined".into()
            }
            Expr::Lam(params, _, body) => {
                // 作った時の可変な変数の値を写し取る（インタプリタと同じ意味にする）
                let muts: Vec<String> = self
                    .scopes
                    .iter()
                    .flat_map(|s| s.values())
                    .filter(|(_, m)| *m)
                    .map(|(js, _)| js.clone())
                    .collect();
                let params: Vec<String> = params.iter().map(|p| p.0.clone()).collect();
                let f = self.function(&params, body);
                self.let_(format!("(({0}) => {f})({0})", muts.join(", ")))
            }
            Expr::Pi(..) => lit("fn"),
            Expr::Refine(..) => lit("refine"),
            Expr::Macro(..) | Expr::Quote(_) | Expr::Method(..) => unreachable!("型検査で弾く"),
        }
    }

    fn match_(&mut self, s: &Expr, arms: &[Arm]) -> String {
        let v = self.expr(s);
        let t = self.tmp();
        self.push(format!("let {t};"));
        self.push(format!("switch ({v}.$c) {{"));
        for arm in arms {
            self.push(if arm.ctor == "_" {
                "default: {".into()
            } else {
                format!("case {}: {{", lit(&arm.ctor))
            });
            self.scopes.push(HashMap::new());
            for (i, b) in arm.binds.iter().enumerate() {
                if b != "_" {
                    let js = self.bind(b, false);
                    self.push(format!("const {js} = {v}.$f[{i}];"));
                }
            }
            let r = self.expr(&arm.body);
            self.scopes.pop();
            self.push(format!("{t} = {r};"));
            self.push("break;".into());
            self.push("}".into());
        }
        self.push("}".into());
        t
    }
}
