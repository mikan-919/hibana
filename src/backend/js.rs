// 型検査を通ったプログラムから型を消し、JavaScript を出力する
//
// 値の表し方: int と number は JavaScript の number（整数は53ビット安全整数）、bool・str は JavaScript の値、
// unit は undefined、Vec は配列、type の値は { $c: 構築子名, $f: [フィールド] }、関数は関数、
// 型は名前の文字列。式は1つずつ一時変数に入れて、評価の順序をインタプリタと揃える
use crate::parser::{Arm, Block, Expr, Fn, Program, Stmt};
use std::collections::HashMap;

const PRELUDE: &str = r#""use strict";
const $SAFE_MAX = 9007199254740991;
function $ichk(x) { if (!Number.isSafeInteger(x)) throw new Error("オーバーフロー"); return x === 0 ? 0 : x; }
function $fchk(x) { if (!Number.isFinite(x)) throw new Error("number の演算結果が有限でない"); return x; }
function $iadd(a, b) { return typeof a === "string" ? a + b : $ichk(a + b); }
function $isub(a, b) { return $ichk(a - b); }
function $imul(a, b) { return $ichk(a * b); }
function $idiv(a, b) { if (b === 0) throw new Error("0 で除算した"); return $ichk(Math.trunc(a / b)); }
function $irem(a, b) { if (b === 0) throw new Error("0 で除算した"); return $ichk(a % b); }
function $ipow(a, b) { if (b < 0) throw new Error("負の指数"); if (b > 52) { if (a > 1 || a < -1) throw new Error("オーバーフロー"); b = 52 + b % 2; } return $ichk(a ** b); }
function $ineg(a) { return $ichk(-a); }
function $fadd(a, b) { return $fchk(a + b); }
function $fsub(a, b) { return $fchk(a - b); }
function $fmul(a, b) { return $fchk(a * b); }
function $fdiv(a, b) { if (b === 0) throw new Error("0 で除算した"); return $fchk(a / b); }
function $frem(a, b) { if (b === 0) throw new Error("0 で除算した"); return $fchk(a % b); }
function $fpow(a, b) {
  if (!Number.isInteger(b)) return $fchk(a ** b);
  let n = Math.abs(b), factor = b < 0 ? $fchk(1 / a) : a, result = 1;
  while (n !== 0) { if (n % 2 !== 0) result = $fchk(result * factor); n = Math.trunc(n / 2); if (n !== 0) factor = $fchk(factor * factor); }
  return result;
}
function $fneg(a) { return $fchk(-a); }
function $num_floor(n) { const x = Math.floor(n); if (!Number.isSafeInteger(x)) throw new Error("floor の結果が int の範囲外"); return x === 0 ? 0 : x; }
function $num_sqrt(n) { if (n < 0) throw new Error("sqrt の引数が負"); return Math.sqrt(n); }
let $w;
// 失敗した式の位置を例外に保持する。内側で付いた位置と、with を抜ける制御はそのまま通す
class $Located extends Error { constructor(e, at) { super(e?.message ?? String(e), { cause: e }); this.at = at; } }
function $at(e, at) { return e instanceof $Located || e instanceof $Abort ? e : new $Located(e, at); }
function $report(e) { console.error("実行時エラー: " + (e?.message ?? String(e)) + (e instanceof $Located ? "（" + e.at + "）" : "")); }
// 書き換えられる参照
class $Ref { constructor(v) { this.v = v; } }
function $d(c, f) { return { $c: c, $f: f }; }
function $field(x, f) { return x.$f[$fi[x.$c][f]]; }
function $eq(a, b) {
  if (a === b) return true;
  if (a instanceof $Ref) return false;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => $eq(x, b[i]));
  if (a && b && typeof a === "object" && typeof b === "object") return a.$c === b.$c && $eq(a.$f, b.$f);
  return false;
}
function $numstr(v) {
  if (Object.is(v, -0)) return "-0";
  const s = String(v);
  if (!s.includes("e")) return s;
  const [mantissa, exponent] = s.split("e"), sign = v < 0 ? "-" : "", m = mantissa.replace("-", "");
  const digits = m.replace(".", ""), point = (m.includes(".") ? m.indexOf(".") : m.length) + (+exponent);
  if (point <= 0) return sign + "0." + "0".repeat(-point) + digits;
  if (point >= digits.length) return sign + digits + "0".repeat(point - digits.length);
  return sign + digits.slice(0, point) + "." + digits.slice(point);
}
function $show(v) {
  if (typeof v === "number") return $numstr(v);
  if (typeof v === "boolean") return String(v);
  if (typeof v === "string") return JSON.stringify(v);
  if (v === undefined) return "()";
  if (typeof v === "function") return "<fn>";
  if (v instanceof $Ref) return "<ref>";
  if (Array.isArray(v)) return "[" + v.map($show).join(", ") + "]";
  const [name, style] = $constructors[v.$c] ?? [v.$c, 0];
  if (style === 2) return name + "(" + v.$f.map($show).join(", ") + ")";
  if (style === 1) return name + " { " + $fields[v.$c].map((f, i) => f + ": " + $show(v.$f[i])).join(", ") + " }";
  return v.$f.length === 0 ? name : name + "(" + v.$f.map($show).join(", ") + ")";
}
// 処理中の handle。内側ほど後ろ。腕の中の操作は、その handle より外で処理する
// 並べ替えずに作り直す（並行に動く処理と共有するため）。await の後は $k で、待つ前の並びに戻す
let $h = [];
function $k(h, v) { $h = h; return v; }
// all。Async<T> の値（Promise を返す引数なしの関数）を全部走らせ、全部終わるのを待つ。最初の失敗で戻り、残りの結果は捨てる
// ponytail: 失敗した時、残りの処理は走り続ける。取り消すには AbortSignal を各処理に通す必要がある
async function $all(fs) {
  const h = $h;
  try { return await Promise.all(fs.map((f) => { $h = h; return f(); })); } finally { $h = h; }
}
// spawn。Async<T> の値を走らせ始め、状態（s: 0 未完了・1 値・2 失敗）を持つ Task を返す。失敗は console.error に出す
function $spawn(f) {
  const h = $h;
  const t = { s: 0, v: undefined, p: null };
  t.p = f().then((v) => { t.s = 1; t.v = v; }, (e) => { t.s = 2; $report(e); });
  $h = h;
  return t;
}
function $then(t, g) { t.p.then(() => { if (t.s === 1) g(t.v); }); }
class $Abort { constructor(h, v) { this.h = h; this.v = v; } }
// 要求の呼び出し。一番内側の with の腕を、その with より外の並びで呼ぶ。腕の値が #resume なら呼んだ所の値、それ以外は with を抜ける
function $find(op) {
  for (let i = $h.length - 1; i >= 0; i--) if (Object.hasOwn($h[i].arms, op)) return i;
  throw new Error("要求 " + op + " を提供する with が無い");
}
function $done(h, r) {
  if (r && r.$c === "$resume") return r.$f[0];
  throw new $Abort(h, r);
}
function $op(op, args) {
  const i = $find(op), h = $h[i], saved = $h;
  $h = $h.slice(0, i);
  let r;
  try { r = h.arms[op](...args); } finally { $h = saved; }
  return $done(h, r);
}
// メソッドの宣言に with Async があるトレイトの腕は async 関数なので、待つ
async function $aop(op, args) {
  const i = $find(op), h = $h[i], saved = $h;
  $h = $h.slice(0, i);
  let r;
  try { r = await h.arms[op](...args); } finally { $h = saved; }
  return $done(h, r);
}
function $str(v) { return typeof v === "string" ? v : $show(v); }
function $print(v) { console.log($str(v)); }
// 標準入力は呼び出し間で共有する区画に読む。区画が空の時だけ readSync を 1 回呼ぶ（対話の入力を待たずに返す）
const $in = { buf: new Uint8Array(4096), pos: 0, end: 0 };
function $input() {
  if (typeof process === "undefined" || !process.getBuiltinModule) throw new Error("input には Node.js の標準入力が必要です");
  const fs = process.getBuiltinModule("node:fs"), bytes = [];
  for (;;) {
    if ($in.pos === $in.end) {
      const n = fs.readSync(0, $in.buf, 0, $in.buf.length, null);
      if (n === 0) break;
      $in.pos = 0;
      $in.end = n;
      continue;
    }
    const nl = $in.buf.subarray($in.pos, $in.end).indexOf(10);
    const stop = nl < 0 ? $in.end : $in.pos + nl;
    for (let i = $in.pos; i < stop; i++) bytes.push($in.buf[i]);
    $in.pos = stop;
    if (nl >= 0) {
      $in.pos++;
      if (bytes.at(-1) === 13) bytes.pop();
      break;
    }
  }
  return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(new Uint8Array(bytes));
}
function $len(v) { return typeof v === "string" ? [...v].length : v.length; }
function $split(s, sep) { return sep === "" ? ["", ...s, ""] : s.split(sep); }
function $starts_with(s, p) { return s.startsWith(p); }
// x = push(x, e)。$o は x の変数だけが指している配列の印で、x を他で読むと $sh が消す
function $sh(v) { if (v !== null && v !== undefined && v.$o) v.$o = false; return v; }
function $opush(v, x) {
  if (v.$o) { v.push(x); return v; }
  const c = [...v, x]; c.$o = true; return c;
}
function $set(v, i, x) { const c = [...v]; c[i] = x; return c; }
function $oset(v, i, x) {
  if (v.$o) { v[i] = x; return v; }
  const c = [...v]; c[i] = x; c.$o = true; return c;
}
function $chars(s) { return [...s]; }
function $code(s) { return s.length ? s.codePointAt(0) : -1; }
function $char_from_codepoint(n) { return n < 0 || n > 0x10ffff || (n >= 0xd800 && n <= 0xdfff) ? "" : String.fromCodePoint(n); }
function $panic(m) { throw new Error(m); }
function $now_ms() { return Date.now(); }
function $slice(s, i, j) {
  const cs = [...s];
  if (!(0 <= i && i <= j && j <= cs.length)) throw new Error(`slice の範囲 ${i}..${j} が文字列の外`);
  return cs.slice(i, j).join("");
}
// extern の関数の境界で、宣言した型 d に従って値を変換する。$to は hibana から JS へ、$from は JS から hibana へ
function $to(d, v) {
  if (d === "number") return $fchk(v);
  if (d === "int") return v;
  if (d.vec) return v.map((x) => $to(d.vec, x));
  if (d.fn && d.async) return (...a) => Promise.resolve(v(...a.map((x, i) => $from(d.fn[i], x)))).then((r) => $to(d.ret, r));
  if (d.fn) return (...a) => $to(d.ret, v(...a.map((x, i) => $from(d.fn[i], x))));
  if (d.st) return Object.fromEntries(d.f.map(([n, fd], i) => [n, $to(fd, v.$f[i])]));
  return v;
}
function $from(d, v) {
  if (d === "number") {
    if (typeof v !== "number") throw new Error("JavaScript の値 " + v + " は number でない");
    return $fchk(v);
  }
  if (d === "int") {
    if (typeof v !== "number" || !Number.isSafeInteger(v)) throw new Error("JavaScript の値 " + v + " は int でない");
    return v === 0 ? 0 : v;
  }
  if (d === "unit") return undefined;
  if (d.vec) return Array.from(v, (x) => $from(d.vec, x));
  if (d.fn && d.async) return async (...a) => $from(d.ret, await v(...a.map((x, i) => $to(d.fn[i], x))));
  if (d.fn) return (...a) => $from(d.ret, v(...a.map((x, i) => $to(d.fn[i], x))));
  if (d.st) return $d(d.st, d.f.map(([n, fd]) => $from(fd, v[n])));
  return v;
}
"#;

fn numeric(e: &Expr) -> bool {
    matches!(e.peel(), Expr::Num(_)) || matches!(e.peel(), Expr::Call(n, _) if n == "#num")
}

pub fn emit(prog: &Program, info: &crate::typeck::Info) -> String {
    let mut g = Gen {
        awaits: &info.awaits,
        var_awaits: &info.var_awaits,
        async_inst: &info.async_inst,
        variant: false,
        exits: HashMap::new(),
        loop_label: None,
        fns: prog.fns.iter().map(|f| (f.name.as_str(), f)).collect(),
        ctors: prog
            .datas
            .iter()
            .flat_map(|d| &d.ctors)
            .map(|c| c.name.as_str())
            .collect(),
        slots: {
            let mut m: HashMap<&str, Option<usize>> = HashMap::new();
            for c in prog.datas.iter().flat_map(|d| &d.ctors) {
                for (i, f) in c.fields.iter().enumerate() {
                    let e = m.entry(f.0.as_str()).or_insert(Some(i));
                    if *e != Some(i) {
                        *e = None;
                    }
                }
            }
            m
        },
        lines: Vec::new(),
        position: None,
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
    let constructors: Vec<_> = prog.datas.iter().flat_map(|data| &data.ctors).map(|ctor| {
        let kind = match ctor.name.structure_style() {
            Some(crate::frontend::name::StructureStyle::Record) => 1,
            Some(crate::frontend::name::StructureStyle::Tuple) => 2,
            None => 0,
        };
        format!("{}: [{}, {kind}]", lit(&ctor.name), lit(&ctor.name.structure_owner().unwrap_or_else(|| ctor.name.clone())))
    }).collect();
    out += &format!("const $constructors = {{ {} }};\n", constructors.join(", "));
    // 構築子ごとの、フィールドの名前から位置への表
    out += "const $fi = Object.fromEntries(Object.entries($fields).map(([c, fs]) => [c, Object.fromEntries(fs.map((f, i) => [f, i]))]));\n";
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
    out += &format!("(async () => {{ try {{ await {}(); }} catch (e) {{ $report(e); if (globalThis.process) process.exitCode = 1; }} }})();\n", ident("f_", "main"));
    out
}

// 型の式を、extern の境界での変換の記述（$to と $from が読む）にする。int・Vec・関数・構造体のほかはそのまま渡す
fn desc(prog: &Program, t: &Expr, depth: usize) -> String {
    if depth > 8 {
        return "\"any\"".into();
    }
    match t.peel() {
        Expr::Var(n) if n == "number" => "\"number\"".into(),
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
                .find(|d| &d.name == n && d.ctors.len() == 1 && d.ctors[0].name.structure_owner().is_some())
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

// 出力先の識別子へ変換する。異なる綴りが同じ名前にならないよう全バイトを符号化する。
fn ident(prefix: &str, n: &str) -> String {
    use std::fmt::Write;
    let mut result = prefix.to_string();
    for byte in n.bytes() { write!(&mut result, "{byte:02x}").unwrap(); }
    result
}

struct Gen<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    ctors: std::collections::HashSet<&'a str>,
    // フィールドの名前から、どの構築子でも同じならその位置（違えば None で、実行時に $field で探す）
    slots: HashMap<&'a str, Option<usize>>,
    // 今の関数の文
    lines: Vec<String>,
    // 位置を持たない生成式（非同期呼び出しの無名関数など）は、元の式の位置を引き継ぐ
    position: Option<crate::lexer::Pos>,
    // hibana の変数名から (JavaScript の名前, 可変か)
    scopes: Vec<HashMap<String, (String, bool)>>,
    next: usize,
    // async 関数にする関数。呼び出しを await する
    async_fns: std::collections::HashSet<&'a str>,
    // 型検査が記録した、待つ呼び出しと async にする無名関数の本体（節点の識別子）
    awaits: &'a std::collections::HashSet<crate::parser::NodeId>,
    // 効果の変数を通して効果を起こす呼び出しと本体。async 版の中でだけ待つ
    var_awaits: &'a std::collections::HashSet<crate::parser::NodeId>,
    // 多相な関数の async 版を呼ぶ呼び出し
    async_inst: &'a std::collections::HashSet<crate::parser::NodeId>,
    // 多相な関数の async 版を出力している
    variant: bool,
    // with（Handle）の識別子から、その handle の JavaScript の変数。exit が $Abort で抜ける先
    exits: HashMap<crate::parser::NodeId, String>,
    loop_label: Option<String>,
}

impl Gen<'_> {
    fn tmp(&mut self) -> String {
        self.next += 1;
        format!("t{}", self.next)
    }

    // 型検査が待つと記録した呼び出しなら待つ
    fn await_(&self, id: Option<crate::parser::NodeId>, call: String) -> String {
        wait(id.is_some_and(|id| self.waits(id)), call)
    }

    fn waits(&self, addr: crate::parser::NodeId) -> bool {
        self.awaits.contains(&addr) || (self.variant && self.var_awaits.contains(&addr))
    }

    // 失敗しうる式か（算術・slice・操作・JavaScript の関数の呼び出し）。例外に位置を付ける所
    fn risky(&self, e: &Expr) -> bool {
        match e.peel() {
            Expr::Binary(op, ..) => matches!(*op, "+" | "-" | "*" | "**" | "/" | "%"),
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
        let saved_loop = self.loop_label.take();
        self.scopes.push(HashMap::new());
        let ps: Vec<String> = params.iter().map(|p| self.bind(p, false)).collect();
        let r = self.block(body);
        self.push(format!("return {r};"));
        self.scopes.pop();
        self.loop_label = saved_loop;
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
        let params: Vec<String> = f.params.iter().map(|p| p.0.to_string()).collect();
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

    // 読むだけで他に渡さない所（len と添字）。変数なら $sh で包まない
    fn look(&mut self, e: &Expr) -> String {
        match e.peel() {
            Expr::Var(n) if self.lookup(n).is_some() => self.lookup(n).unwrap().0.clone(),
            _ => self.expr(e),
        }
    }

    fn args(&mut self, args: &[Expr]) -> Vec<String> {
        args.iter().map(|a| self.expr(a)).collect()
    }

    // 位置の包みと算術・比較は、深く入れ子になるので大きな expr_inner のフレームを通らずに生成する
    fn expr(&mut self, e: &Expr) -> String {
        self.expr_with_id(e, None)
    }

    fn expr_with_id(&mut self, e: &Expr, id: Option<crate::parser::NodeId>) -> String {
        match e {
            Expr::At(sp, x) => {
                let saved = self.position.replace(sp.start);
                let v = self.expr_with_id(x, id);
                self.position = saved;
                v
            }
            Expr::Id(id, inner) => self.expr_with_id(inner, Some(*id)),
            Expr::InModule(_, inner) => self.expr_with_id(inner, id),
            _ => {
                if let Some(p) = self.position.filter(|_| self.risky(e)) {
                    let t = self.tmp();
                    self.push(format!("let {t};"));
                    self.push("try {".into());
                    let v = self.expr_unlocated(e, id);
                    self.push(format!("{t} = {v};"));
                    self.push(format!("}} catch (e) {{ throw $at(e, \"{} 行 {} 列\"); }}", p.line, p.column));
                    t
                } else {
                    self.expr_unlocated(e, id)
                }
            }
        }
    }

    fn expr_unlocated(&mut self, e: &Expr, id: Option<crate::parser::NodeId>) -> String {
        match e {
            // && と || は右辺を条件付きで生成するので expr_inner で扱う
            Expr::Binary(op, l, r) if !matches!(*op, "&&" | "||") => self.binary(op, l, r),
            _ => self.expr_inner(e, id),
        }
    }

    #[inline(never)]
    fn binary(&mut self, op: &str, l: &Expr, r: &Expr) -> String {
        let prefix = if numeric(l) || numeric(r) { "f" } else { "i" };
        let (a, b) = (self.expr(l), self.expr(r));
        let v = match op {
            "+" => format!("${prefix}add({a}, {b})"),
            "-" => format!("${prefix}sub({a}, {b})"),
            "*" => format!("${prefix}mul({a}, {b})"),
            "/" => format!("${prefix}div({a}, {b})"),
            "%" => format!("${prefix}rem({a}, {b})"),
            "**" => format!("${prefix}pow({a}, {b})"),
            "==" => format!("$eq({a}, {b})"),
            "!=" => format!("!$eq({a}, {b})"),
            op => format!("({a} {op} {b})"),
        };
        self.let_(v)
    }

    #[inline(never)]
    fn expr_inner(&mut self, e: &Expr, id: Option<crate::parser::NodeId>) -> String {
        match e {
            Expr::At(..) | Expr::Id(..) | Expr::InModule(..) => unreachable!("expr で外している"),
            Expr::Int(n) => n.to_string(),
            Expr::Num(n) => format!("{n:?}"),
            Expr::Bool(b) => b.to_string(),
            Expr::Str(s) => lit(s),
            Expr::Var(n) => match self.lookup(n) {
                Some((js, true)) => format!("$sh({js})"),
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
                    format!("${}neg({a})", if numeric(x) { "f" } else { "i" })
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
            Expr::Binary(..) => unreachable!("expr で分けている"),
            Expr::Call(name, args) => {
                if let Some((js, _)) = self.lookup(name).cloned() {
                    let vs = self.args(args);
                    let v = self.await_(id, format!("{js}({})", vs.join(", ")));
                    return self.let_(v);
                }
                let mut vs = if name == "len" { vec![self.look(&args[0])] } else { self.args(args) };
                let v = match name.as_str() {
                    "#num" => vs[0].clone(),
                    "num_floor" | "num_sqrt" => format!("${name}({})", vs[0]),
                    // 腕の値として返し、$op が呼んだ所の値にする
                    "#resume" => format!(
                        "$d(\"$resume\", [{}])",
                        vs.first().map_or("undefined", |v| v)
                    ),
                    n if self.fns.get(n).is_some_and(|f| f.op.is_some()) => {
                        let f = self.fns[n];
                        let k = vs.len() - (f.params.len() - f.implicit);
                        // メソッドの宣言に with Async があれば、腕を待つ
                        if is_async(f) {
                            wait(true, format!("$aop({}, [{}])", lit(n), vs[k..].join(", ")))
                        } else {
                            format!("$op({}, [{}])", lit(n), vs[k..].join(", "))
                        }
                    }
                    "new_ref" => format!("new $Ref({})", vs[0]),
                    "read" => format!("{}.v", vs[0]),
                    "write" => format!("($w = {}, $w.v = {}, undefined)", vs[0], vs[1]),
                    "push" => format!("[...{}, {}]", vs[0], vs[1]),
                    "set_at" => format!("$set({}, {}, {})", vs[0], vs[1], vs[2]),
                    "all" => wait(true, format!("$all({})", vs[0])),
                    "spawn" => format!("$spawn({})", vs[0]),
                    "task_then" => format!("$then({}, {})", vs[0], vs[1]),
                    "task_done" => format!("({}.s === 1)", vs[0]),
                    "task_value" | "task_wait" => format!("{}.v", vs[0]),
                    "put" => format!("$print({})", vs[0]),
                    "input" => "$input()".into(),
                    "to_str" => format!("$str({})", vs[0]),
                    "len" | "split" | "starts_with" | "slice" | "chars" | "code" | "char_from_codepoint" | "panic" | "now_ms" => {
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
                            if id.is_some_and(|id| self.async_inst.contains(&id)
                                || (self.variant && is_poly(f) && self.var_awaits.contains(&id)))
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
                let v = self.await_(id, format!("{f}({})", vs.join(", ")));
                self.let_(v)
            }
            Expr::Field(x, f) => {
                let a = self.expr(x);
                match self.slots.get(f.as_str()) {
                    Some(Some(k)) => self.let_(format!("{a}.$f[{k}]")),
                    _ => self.let_(format!("$field({a}, {})", lit(f))),
                }
            }
            Expr::Index(v, i) => {
                let (a, b) = (self.look(v), self.expr(i));
                self.let_(format!("{a}[{b}]"))
            }
            Expr::Array(items) => {
                let vs = self.args(items);
                self.let_(format!("[{}]", vs.join(", ")))
            }
            Expr::Assign(n, x) => {
                let js = self.lookup(n).expect("型検査済み").0.clone();
                let v = match (x.self_push(n), x.self_set(n)) {
                    (Some(e), _) if !self.fns.contains_key("push") => format!("$opush({js}, {})", self.expr(e)),
                    (_, Some((i, e))) if !self.fns.contains_key("set_at") => {
                        let i = self.expr(i);
                        format!("$oset({js}, {i}, {})", self.expr(e))
                    }
                    _ => self.expr(x),
                };
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
                let label = self.tmp();
                self.push(format!("{label}: while (true) {{"));
                let c = self.expr(c);
                self.push(format!("if (!{c}) break;"));
                let saved = self.loop_label.replace(label);
                self.block(body);
                self.loop_label = saved;
                self.push("}".into());
                "undefined".into()
            }
            Expr::For(x, it, end, body) => {
                let a = self.expr(it);
                let head = match end {
                    Some(end) => {
                        let a = self.let_(a);
                        let b = self.expr(end);
                        let b = self.let_(b);
                        let i = self.tmp();
                        (format!("for (let {i} = {a}; {i} < {b}; {i}++) {{"), i)
                    }
                    None => {
                        let i = self.tmp();
                        (format!("for (const {i} of {a}) {{"), i)
                    }
                };
                let label = self.tmp();
                self.push(format!("{label}: {}", head.0));
                self.scopes.push(HashMap::new());
                let js = self.bind(x, false);
                self.push(format!("const {js} = {};", head.1));
                let saved = self.loop_label.replace(label);
                self.block(body);
                self.loop_label = saved;
                self.scopes.pop();
                self.push("}".into());
                "undefined".into()
            }
            Expr::Break | Expr::Continue => {
                let keyword = if matches!(e, Expr::Break) { "break" } else { "continue" };
                let label = self.loop_label.as_deref().expect("ループの本体の中だけ");
                self.push(format!("{keyword} {label};"));
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
                let params: Vec<String> = params.iter().map(|p| p.0.to_string()).collect();
                let a = self.waits(body.id);
                let f = self.function(&params, body, a);
                let shared: Vec<String> = muts.iter().map(|m| format!("$sh({m})")).collect();
                self.let_(format!("(({}) => {f})({})", muts.join(", "), shared.join(", ")))
            }
            Expr::Handle(node, body, arms) => {
                // 腕は無名関数にする。本体はその場に書き、exit した腕の値を例外で受け取る
                let h = self.let_("{ arms: null }".into());
                self.exits.insert(*node, h.clone());
                let fs: Vec<String> = arms
                    .iter()
                    .map(|a| {
                        let params = a.binds.iter().map(|b| (b.clone(), None)).collect();
                        let body = match &a.body {
                            Expr::Block(b) => b.clone(),
                            e => Block { id: Default::default(), stmts: vec![], tail: Some(Box::new(e.clone())) },
                        };
                        let f = self.expr(&Expr::Lam(params, None, None, body));
                        format!("{}: {f}", lit(&a.ctor))
                    })
                    .collect();
                self.push(format!("{h}.arms = {{ {} }};", fs.join(", ")));
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
            Expr::Exit(target, x) => {
                let v = self.expr(x);
                let h = self.exits[&target.expect("型検査が行き先を書き込む")].clone();
                self.push(format!("throw new $Abort({h}, {v});"));
                "undefined".into()
            }
            Expr::Pi(..) => lit("fn"),
            Expr::Refine(..) => lit("refine"),
            Expr::Macro(..) | Expr::Quote(_) | Expr::Method(..) | Expr::Await(_) | Expr::Try(_) => unreachable!("型検査で弾く・書き換える"),
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
