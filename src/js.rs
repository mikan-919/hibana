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
let $w;
// 最後に通った、失敗しうる式の位置
let $at;
// 書き換えられる参照
class $Ref { constructor(v) { this.v = v; } }
function $d(c, f) { return { $c: c, $f: f }; }
function $field(x, f) { return x.$f[$fields[x.$c].indexOf(f)]; }
function $eq(a, b) {
  if (a === b) return true;
  if (a instanceof $Ref) return false;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => $eq(x, b[i]));
  if (a && b && typeof a === "object" && typeof b === "object") return a.$c === b.$c && $eq(a.$f, b.$f);
  return false;
}
function $show(v) {
  if (typeof v === "bigint" || typeof v === "boolean") return String(v);
  if (typeof v === "string") return JSON.stringify(v);
  if (v === undefined) return "()";
  if (typeof v === "function") return "<fn>";
  if (v instanceof $Ref) return "<ref>";
  if (Array.isArray(v)) return "[" + v.map($show).join(", ") + "]";
  if (v.$c.endsWith("{}") && $fields[v.$c][0] === "0") return v.$c.slice(0, -2) + "(" + v.$f.map($show).join(", ") + ")";
  if (v.$c.endsWith("{}")) return v.$c.slice(0, -2) + " { " + $fields[v.$c].map((f, i) => f + ": " + $show(v.$f[i])).join(", ") + " }";
  return v.$f.length === 0 ? v.$c : v.$c + "(" + v.$f.map($show).join(", ") + ")";
}
// web ライブラリの serve と worker。app は JS の関数（$to で変換済み）で、{ method, path, body } を受け取る
globalThis.hibana_serve = async (port, app) => {
  const http = await import("node:http");
  const server = http.createServer(async (req, res) => {
    try {
      let body = "";
      for await (const c of req) body += c;
      const u = new URL(req.url, "http://localhost");
      const headers = Object.entries(req.headers).map(([name, value]) => ({ name, value: String(value) }));
      const r = await app({ method: req.method, path: u.pathname, query: u.search.slice(1), headers, body });
      res.writeHead(r.status, { "content-type": r.content_type });
      res.end(r.body);
    } catch (e) {
      console.error("実行時エラー: " + e.message + ($at ? "（" + $at + "）" : ""));
      res.writeHead(500, { "content-type": "text/plain" });
      res.end("Internal Server Error");
    }
  });
  await new Promise((ok) => server.listen(port, ok));
  console.log("http://localhost:" + server.address().port);
};
let $worker;
globalThis.hibana_worker = (app) => { $worker = app; };
const $fetch = async (req) => {
  const u = new URL(req.url);
  const headers = [...req.headers].map(([name, value]) => ({ name, value }));
  const r = await $worker({ method: req.method, path: u.pathname, query: u.search.slice(1), headers, body: await req.text() });
  return new Response(r.body, { status: r.status, headers: { "content-type": r.content_type } });
};
// 処理中の handle。内側ほど後ろ。腕の中の操作は、その handle より外で処理する
// 並べ替えずに作り直す（並行に動く処理と共有するため）。await の後は $k で、待つ前の並びに戻す
let $h = [];
function $k(h, v) { $h = h; return v; }
// all。すべて終わるのを待ち、失敗したものがあれば最初の失敗を投げる
// ponytail: 失敗しても残りを中断しない（終わるまで待つだけ）。中断するなら AbortSignal を各処理に渡す
async function $all(fs) {
  const h = $h;
  const rs = await Promise.allSettled(fs.map((f) => { $h = h; return f(); }));
  $h = h;
  const bad = rs.find((r) => r.status === "rejected");
  if (bad) throw bad.reason;
  return rs.map((r) => r.value);
}
class $Abort { constructor(h, v) { this.h = h; this.v = v; } }
function $op(op, args) {
  for (let i = $h.length - 1; i >= 0; i--) {
    const h = $h[i];
    if (!Object.hasOwn(h.arms, op)) continue;
    const saved = $h;
    $h = $h.slice(0, i);
    let r;
    try { r = h.arms[op](...args); } finally { $h = saved; }
    if (r && r.$c === "$resume") return r.$f[0];
    throw new $Abort(h, r);
  }
  throw new Error("操作 " + op + " を処理する handle が無い");
}
function $str(v) { return typeof v === "string" ? v : $show(v); }
function $print(v) { console.log($str(v)); }
function $len(v) { return BigInt(typeof v === "string" ? [...v].length : v.length); }
function $split(s, sep) { return s.split(sep); }
function $starts_with(s, p) { return s.startsWith(p); }
function $chars(s) { return [...s]; }
function $code(s) { return s.length ? BigInt(s.codePointAt(0)) : -1n; }
function $panic(m) { throw new Error(m); }
function $slice(s, i, j) {
  const cs = [...s];
  if (!(0n <= i && i <= j && j <= BigInt(cs.length))) throw new Error(`slice の範囲 ${i}..${j} が文字列の外`);
  return cs.slice(Number(i), Number(j)).join("");
}
// extern の関数の境界で、宣言した型 d に従って値を変換する。$to は hibana から JS へ、$from は JS から hibana へ
function $to(d, v) {
  if (d === "int") return Number(v);
  if (d.vec) return v.map((x) => $to(d.vec, x));
  if (d.fn && d.async) return (...a) => Promise.resolve(v(...a.map((x, i) => $from(d.fn[i], x)))).then((r) => $to(d.ret, r));
  if (d.fn) return (...a) => $to(d.ret, v(...a.map((x, i) => $from(d.fn[i], x))));
  if (d.st) return Object.fromEntries(d.f.map(([n, fd], i) => [n, $to(fd, v.$f[i])]));
  return v;
}
function $from(d, v) {
  if (d === "int") {
    if (typeof v === "bigint") return $chk(v);
    if (!Number.isInteger(v)) throw new Error("JavaScript の値 " + v + " は int でない");
    return BigInt(v);
  }
  if (d === "unit") return undefined;
  if (d.vec) return Array.from(v, (x) => $from(d.vec, x));
  if (d.fn && d.async) return async (...a) => $from(d.ret, await v(...a.map((x, i) => $to(d.fn[i], x))));
  if (d.fn) return (...a) => $from(d.ret, v(...a.map((x, i) => $to(d.fn[i], x))));
  if (d.st) return $d(d.st, d.f.map(([n, fd]) => $from(fd, v[n])));
  return v;
}
"#;

pub fn emit(prog: &Program, info: &crate::typeck::Info) -> String {
    let mut g = Gen {
        awaits: &info.awaits,
        var_awaits: &info.var_awaits,
        async_inst: &info.async_inst,
        variant: false,
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
        async_fns: prog
            .fns
            .iter()
            .filter(|f| is_async(f))
            .map(|f| f.name.as_str())
            .collect(),
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
    let mut imports = String::new();
    for f in &prog.fns {
        match &f.js {
            Some(m) => {
                let (import, def) = g.extern_fn(prog, f, m);
                imports += &import;
                out += &def;
            }
            None => out += &g.func(f),
        }
    }
    // import は ES モジュールの先頭に置く
    let mut out = imports + &out;
    // web を使うプログラムは Cloudflare Workers の ES モジュールとしても使える
    if g.fns.contains_key("web.worker") {
        out += "export default { fetch: $fetch };\n";
    }
    out += "(async () => { try { await f_main(); } catch (e) { console.error(\"実行時エラー: \" + e.message + ($at ? \"（\" + $at + \"）\" : \"\")); if (globalThis.process) process.exitCode = 1; } })();\n";
    out
}

// 型の式を、extern の境界での変換の記述（$to と $from が読む）にする。int・Vec・関数・構造体のほかはそのまま渡す
fn desc(prog: &Program, t: &Expr, depth: usize) -> String {
    if depth > 8 {
        return "\"any\"".into();
    }
    match t.peel() {
        Expr::Var(n) if n == "int" || n == "unit" => lit(n),
        Expr::Refine(b, ..) => desc(prog, b, depth + 1),
        Expr::Call(n, args) if n == "Vec" && args.len() == 1 => {
            format!("{{ vec: {} }}", desc(prog, &args[0], depth + 1))
        }
        Expr::Pi(bs, r, effs) => {
            let ps: Vec<String> = bs.iter().map(|(_, t)| desc(prog, t, depth + 1)).collect();
            format!(
                "{{ fn: [{}], ret: {}, async: {} }}",
                ps.join(", "),
                desc(prog, r, depth + 1),
                has_async(effs)
            )
        }
        Expr::Var(n) | Expr::Call(n, _) => {
            // 構造体はフィールドごとに、引数の無い別名はその中身で変換する
            if let Some(d) = prog
                .datas
                .iter()
                .find(|d| &d.name == n && d.ctors.len() == 1 && d.ctors[0].name.ends_with("{}"))
            {
                let c = &d.ctors[0];
                let fs: Vec<String> = c
                    .fields
                    .iter()
                    .map(|(f, t)| format!("[{}, {}]", lit(f), desc(prog, t, depth + 1)))
                    .collect();
                return format!("{{ st: {}, f: [{}] }}", lit(&c.name), fs.join(", "));
            }
            match prog
                .fns
                .iter()
                .find(|f| &f.name == n && f.alias && f.params.is_empty())
            {
                Some(f) => f
                    .body
                    .tail
                    .as_ref()
                    .map_or("\"any\"".into(), |b| desc(prog, b, depth + 1)),
                None => "\"any\"".into(),
            }
        }
        _ => "\"any\"".into(),
    }
}

// Async の効果を起こす関数は、JavaScript の async 関数にする
fn is_async(f: &Fn) -> bool {
    has_async(&f.effects)
}

// 効果の変数（<E: Effect>）を持つ関数
fn is_poly(f: &Fn) -> bool {
    f.params[..f.implicit]
        .iter()
        .any(|(_, t)| matches!(t.peel(), Expr::Var(k) if k == "Effect"))
}

// 待つ呼び出し。待った後に、待つ前の handle の並びに戻す
fn wait(yes: bool, call: String) -> String {
    if yes {
        format!("$k($h, await {call})")
    } else {
        call
    }
}

fn has_async(effs: &[Expr]) -> bool {
    effs.iter()
        .any(|e| matches!(e.peel(), Expr::Var(n) if n == "Async"))
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
    // async 関数にする関数。呼び出しを await する
    async_fns: std::collections::HashSet<&'a str>,
    // 型検査が記録した、待つ呼び出しと async にする無名関数の本体（アドレス）
    awaits: &'a std::collections::HashSet<usize>,
    // 効果の変数を通して効果を起こす呼び出しと本体。async 版の中でだけ待つ
    var_awaits: &'a std::collections::HashSet<usize>,
    // 多相な関数の async 版を呼ぶ呼び出し
    async_inst: &'a std::collections::HashSet<usize>,
    // 多相な関数の async 版を出力している
    variant: bool,
}

impl Gen<'_> {
    fn tmp(&mut self) -> String {
        self.next += 1;
        format!("t{}", self.next)
    }

    // 型検査が待つと記録した呼び出しなら待つ
    fn await_(&self, e: &Expr, call: String) -> String {
        wait(self.waits(e as *const Expr as usize), call)
    }

    fn waits(&self, addr: usize) -> bool {
        self.awaits.contains(&addr) || (self.variant && self.var_awaits.contains(&addr))
    }

    // 失敗しうる式か（算術・slice・操作・JavaScript の関数の呼び出し）。実行時エラーの位置を記録する所
    fn risky(&self, e: &Expr) -> bool {
        match e {
            Expr::Binary(op, ..) => matches!(*op, "+" | "-" | "*" | "/" | "%"),
            Expr::Unary(op, _) => *op == "-",
            Expr::Call(n, _) => {
                n == "slice"
                    || n == "panic"
                    || self
                        .fns
                        .get(n.as_str())
                        .is_some_and(|f| f.op.is_some() || f.js.is_some())
            }
            _ => false,
        }
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
    fn function(&mut self, params: &[String], body: &Block, is_async: bool) -> String {
        let saved = std::mem::take(&mut self.lines);
        self.scopes.push(HashMap::new());
        let ps: Vec<String> = params.iter().map(|p| self.bind(p, false)).collect();
        let r = self.block(body);
        self.push(format!("return {r};"));
        self.scopes.pop();
        let lines = std::mem::replace(&mut self.lines, saved);
        format!(
            "{}function({}) {{\n{}\n}}",
            if is_async { "async " } else { "" },
            ps.join(", "),
            lines.join("\n")
        )
    }

    // extern の関数。(import 文, hibana の値を変換して呼ぶ関数)
    fn extern_fn(&mut self, prog: &Program, f: &Fn, module: &str) -> (String, String) {
        let js_name = f.name.rsplit('.').next().unwrap();
        let imp = ident("$imp_", &f.name);
        let import = if module.is_empty() {
            format!(
                "const {imp} = (...a) => globalThis[{}](...a);\n",
                lit(js_name)
            )
        } else {
            format!("import {{ {js_name} as {imp} }} from {};\n", lit(module))
        };
        let params: Vec<String> = (0..f.params.len()).map(|i| format!("a{i}")).collect();
        let args: Vec<String> = f
            .params
            .iter()
            .zip(&params)
            .map(|((_, t), a)| format!("$to({}, {a})", desc(prog, t, 0)))
            .collect();
        let ret = f
            .ret
            .as_ref()
            .map_or("\"unit\"".to_string(), |t| desc(prog, t, 0));
        let (a, w) = if is_async(f) {
            ("async ", "await ")
        } else {
            ("", "")
        };
        let def = format!(
            "const {} = {a}function({}) {{ return $from({ret}, {w}{imp}({})); }};\n",
            ident("f_", &f.name),
            params.join(", "),
            args.join(", ")
        );
        (import, def)
    }

    fn func(&mut self, f: &Fn) -> String {
        let params: Vec<String> = f.params.iter().map(|p| p.0.clone()).collect();
        let mut s = format!(
            "const {} = {};\n",
            ident("f_", &f.name),
            self.function(&params, &f.body, is_async(f))
        );
        // 効果の多相な関数は、効果の変数に Async が入った時に呼ぶ async 版も出力する（効果の変数を通した呼び出しを待つ）
        if is_poly(f) {
            self.variant = true;
            s += &format!(
                "const {}$a = {};\n",
                ident("f_", &f.name),
                self.function(&params, &f.body, true)
            );
            self.variant = false;
        }
        s
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
            Expr::At(sp, x) => {
                if self.risky(x) {
                    let p = sp.start;
                    self.push(format!("$at = \"{} 行 {} 列\";", p.line, p.column));
                }
                self.expr(x)
            }
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
                    let v = self.await_(e, format!("{js}({})", vs.join(", ")));
                    return self.let_(v);
                }
                let mut vs = self.args(args);
                let v = match name.as_str() {
                    // 腕の値として返し、$op が操作の戻り値にする
                    "resume" => format!(
                        "$d(\"$resume\", [{}])",
                        vs.first().map_or("undefined", |v| v)
                    ),
                    n if self.fns.get(n).is_some_and(|f| f.op.is_some()) => {
                        let f = self.fns[n];
                        let k = vs.len() - (f.params.len() - f.implicit);
                        format!("$op({}, [{}])", lit(n), vs[k..].join(", "))
                    }
                    "new_ref" => format!("new $Ref({})", vs[0]),
                    "read" => format!("{}.v", vs[0]),
                    "write" => format!("($w = {}, $w.v = {}, undefined)", vs[0], vs[1]),
                    "push" => format!("[...{}, {}]", vs[0], vs[1]),
                    "all" => wait(true, format!("$all({})", vs[0])),
                    "print" => format!("$print({})", vs[0]),
                    "to_str" => format!("$str({})", vs[0]),
                    "len" | "split" | "starts_with" | "slice" | "chars" | "code" | "panic" => {
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
                            let addr = e as *const Expr as usize;
                            if self.async_inst.contains(&addr)
                                || (self.variant && is_poly(f) && self.var_awaits.contains(&addr))
                            {
                                wait(true, format!("{}$a({})", ident("f_", n), vs.join(", ")))
                            } else {
                                wait(
                                    self.async_fns.contains(n),
                                    format!("{}({})", ident("f_", n), vs.join(", ")),
                                )
                            }
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
                let v = self.await_(e, format!("{f}({})", vs.join(", ")));
                self.let_(v)
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
            Expr::Lam(params, _, _, body) => {
                // 作った時の可変な変数の値を写し取る（インタプリタと同じ意味にする）
                let muts: Vec<String> = self
                    .scopes
                    .iter()
                    .flat_map(|s| s.values())
                    .filter(|(_, m)| *m)
                    .map(|(js, _)| js.clone())
                    .collect();
                let params: Vec<String> = params.iter().map(|p| p.0.clone()).collect();
                let a = self.waits(body as *const Block as usize);
                let f = self.function(&params, body, a);
                self.let_(format!("(({0}) => {f})({0})", muts.join(", ")))
            }
            Expr::Handle(body, arms) => {
                // 腕は無名関数にする。本体はその場に書き、resume しなかった腕の値を例外で受け取る
                let fs: Vec<String> = arms
                    .iter()
                    .map(|a| {
                        let params = a.binds.iter().map(|b| (b.clone(), None)).collect();
                        let body = Block {
                            stmts: vec![],
                            tail: Some(Box::new(a.body.clone())),
                        };
                        let f = self.expr(&Expr::Lam(params, None, vec![], body));
                        format!("{}: {f}", lit(&a.ctor))
                    })
                    .collect();
                let h = self.let_(format!("{{ arms: {{ {} }} }}", fs.join(", ")));
                let t = self.tmp();
                self.push(format!("let {t};"));
                let saved = self.tmp();
                self.push(format!("const {saved} = $h; $h = [...$h, {h}];"));
                self.push("try {".into());
                let v = self.expr(body);
                self.push(format!("{t} = {v};"));
                self.push(format!("}} catch (e) {{ if (e instanceof $Abort && e.h === {h}) {t} = e.v; else throw e; }} finally {{ $h = {saved}; }}"));
                t
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
