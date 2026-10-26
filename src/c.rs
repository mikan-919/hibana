// 型検査を通ったプログラムから型を消し、C を出力する（ネイティブ向け）
//
// 値はタグ付きの struct V。int は int64_t（オーバーフローは実行時エラー）、文字列・配列・type の値・関数は
// ヒープに置く。関数の値は (コード, 写し取った変数) の組で、無名関数は見えている局所変数をすべて写し取る。
// 式は1つずつ一時変数に入れて、評価の順序をインタプリタと揃える
// ponytail: メモリを解放しない。長く動くプログラムには Boehm GC などを入れる
use crate::parser::{Arm, Block, Expr, Fn, Program, Stmt};
use std::collections::HashMap;

const PRELUDE: &str = r#"#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>
enum { T_UNIT, T_INT, T_BOOL, T_STR, T_ARR, T_DATA, T_FN, T_TY };
typedef struct V V;
struct V { int tag; union { int64_t i; bool b; struct Str* s; struct Arr* a; struct Data* d; struct Fn* f; const char* ty; }; };
typedef struct Str { int64_t len; char s[]; } Str;
typedef struct Arr { int64_t n; V items[]; } Arr;
typedef struct Data { int ctor; int n; V f[]; } Data;
typedef V (*Code)(V* env, V* args);
typedef struct Fn { Code code; int n; V env[]; } Fn;
"#;

const RUNTIME: &str = r#"
static V U = { .tag = T_UNIT };
static void die(const char* m) { fflush(stdout); fprintf(stderr, "実行時エラー: %s\n", m); exit(1); }
static void* alloc(size_t n) { void* p = malloc(n ? n : 1); if (!p) die("メモリが足りない"); return p; }
static V mk_int(int64_t i) { V v = { .tag = T_INT }; v.i = i; return v; }
static V mk_bool(bool b) { V v = { .tag = T_BOOL }; v.b = b; return v; }
static V mk_ty(const char* t) { V v = { .tag = T_TY }; v.ty = t; return v; }
static V mk_str(const char* s, int64_t len) {
  Str* p = alloc(sizeof(Str) + len + 1); p->len = len; memcpy(p->s, s, len); p->s[len] = 0;
  V v = { .tag = T_STR }; v.s = p; return v;
}
static V mk_arr(int64_t n, const V* items) {
  Arr* p = alloc(sizeof(Arr) + n * sizeof(V)); p->n = n; if (n) memcpy(p->items, items, n * sizeof(V));
  V v = { .tag = T_ARR }; v.a = p; return v;
}
static V mk_data(int ctor, int n, const V* fs) {
  Data* p = alloc(sizeof(Data) + n * sizeof(V)); p->ctor = ctor; p->n = n; if (n) memcpy(p->f, fs, n * sizeof(V));
  V v = { .tag = T_DATA }; v.d = p; return v;
}
static V mk_fn(Code code, int n, const V* env) {
  Fn* p = alloc(sizeof(Fn) + n * sizeof(V)); p->code = code; p->n = n; if (n) memcpy(p->env, env, n * sizeof(V));
  V v = { .tag = T_FN }; v.f = p; return v;
}
static V h_call(V f, V* args) { return f.f->code(f.f->env, args); }
static V h_add(V a, V b) {
  if (a.tag == T_STR) {
    Str* p = alloc(sizeof(Str) + a.s->len + b.s->len + 1); p->len = a.s->len + b.s->len;
    memcpy(p->s, a.s->s, a.s->len); memcpy(p->s + a.s->len, b.s->s, b.s->len); p->s[p->len] = 0;
    V v = { .tag = T_STR }; v.s = p; return v;
  }
  int64_t r; if (__builtin_add_overflow(a.i, b.i, &r)) die("オーバーフロー"); return mk_int(r);
}
static V h_sub(V a, V b) { int64_t r; if (__builtin_sub_overflow(a.i, b.i, &r)) die("オーバーフロー"); return mk_int(r); }
static V h_mul(V a, V b) { int64_t r; if (__builtin_mul_overflow(a.i, b.i, &r)) die("オーバーフロー"); return mk_int(r); }
static V h_div(V a, V b) { if (b.i == 0) die("0 で除算した"); if (a.i == INT64_MIN && b.i == -1) die("オーバーフロー"); return mk_int(a.i / b.i); }
static V h_rem(V a, V b) { if (b.i == 0) die("0 で除算した"); if (a.i == INT64_MIN && b.i == -1) die("オーバーフロー"); return mk_int(a.i % b.i); }
static V h_neg(V a) { if (a.i == INT64_MIN) die("オーバーフロー"); return mk_int(-a.i); }
static bool eq(V a, V b) {
  if (a.tag != b.tag) return false;
  switch (a.tag) {
    case T_UNIT: return true;
    case T_INT: return a.i == b.i;
    case T_BOOL: return a.b == b.b;
    case T_STR: return a.s->len == b.s->len && memcmp(a.s->s, b.s->s, a.s->len) == 0;
    case T_TY: return strcmp(a.ty, b.ty) == 0;
    case T_FN: return a.f == b.f;
    case T_ARR:
      if (a.a->n != b.a->n) return false;
      for (int64_t k = 0; k < a.a->n; k++) if (!eq(a.a->items[k], b.a->items[k])) return false;
      return true;
    default:
      if (a.d->ctor != b.d->ctor) return false;
      for (int k = 0; k < a.d->n; k++) if (!eq(a.d->f[k], b.d->f[k])) return false;
      return true;
  }
}
static V h_eq(V a, V b) { return mk_bool(eq(a, b)); }
typedef struct { char* s; size_t n, cap; } Buf;
static void put(Buf* b, const char* s, size_t n) {
  if (b->n + n + 1 > b->cap) { b->cap = (b->n + n + 1) * 2; char* p = alloc(b->cap); if (b->s) memcpy(p, b->s, b->n); b->s = p; }
  memcpy(b->s + b->n, s, n); b->n += n; b->s[b->n] = 0;
}
static void puts_(Buf* b, const char* s) { put(b, s, strlen(s)); }
static void show(Buf* b, V v) {
  char tmp[32];
  switch (v.tag) {
    case T_UNIT: puts_(b, "()"); break;
    case T_INT: snprintf(tmp, sizeof tmp, "%lld", (long long)v.i); puts_(b, tmp); break;
    case T_BOOL: puts_(b, v.b ? "true" : "false"); break;
    case T_TY: puts_(b, v.ty); break;
    case T_FN: puts_(b, "<fn>"); break;
    case T_STR:
      puts_(b, "\"");
      for (int64_t k = 0; k < v.s->len; k++) {
        char c = v.s->s[k];
        switch (c) {
          case '"': puts_(b, "\\\""); break; case '\\': puts_(b, "\\\\"); break;
          case '\n': puts_(b, "\\n"); break; case '\t': puts_(b, "\\t"); break;
          case '\r': puts_(b, "\\r"); break; case 0: puts_(b, "\\0"); break;
          default: put(b, &c, 1);
        }
      }
      puts_(b, "\"");
      break;
    case T_ARR:
      puts_(b, "[");
      for (int64_t k = 0; k < v.a->n; k++) { if (k) puts_(b, ", "); show(b, v.a->items[k]); }
      puts_(b, "]");
      break;
    default: {
      const char* name = CTOR_NAMES[v.d->ctor];
      size_t len = strlen(name);
      if (len >= 2 && strcmp(name + len - 2, "{}") == 0) {
        put(b, name, len - 2); puts_(b, " { ");
        for (int k = 0; k < v.d->n; k++) { if (k) puts_(b, ", "); puts_(b, CTOR_FIELDS[v.d->ctor][k]); puts_(b, ": "); show(b, v.d->f[k]); }
        puts_(b, " }");
      } else if (v.d->n == 0) {
        puts_(b, name);
      } else {
        puts_(b, name); puts_(b, "(");
        for (int k = 0; k < v.d->n; k++) { if (k) puts_(b, ", "); show(b, v.d->f[k]); }
        puts_(b, ")");
      }
    }
  }
}
static V h_str(V v) {
  if (v.tag == T_STR) return v;
  Buf b = {0}; if (v.tag == T_TY) puts_(&b, v.ty); else show(&b, v);
  return mk_str(b.s ? b.s : "", b.n);
}
static V h_print(V v) {
  V s = h_str(v); fwrite(s.s->s, 1, s.s->len, stdout); fputc('\n', stdout); return U;
}
static int64_t utf8_len(const char* s, int64_t n) { int64_t c = 0; for (int64_t k = 0; k < n; k++) if ((s[k] & 0xC0) != 0x80) c++; return c; }
static V h_len(V v) { return mk_int(v.tag == T_STR ? utf8_len(v.s->s, v.s->len) : v.a->n); }
static V h_starts_with(V s, V p) { return mk_bool(s.s->len >= p.s->len && memcmp(s.s->s, p.s->s, p.s->len) == 0); }
// ponytail: 区切りが空文字列の時の振る舞いはインタプリタ（Rust の split）と違う
static V h_split(V s, V sep) {
  int64_t cap = 4, n = 0; V* parts = alloc(cap * sizeof(V));
  const char* p = s.s->s; const char* end = p + s.s->len; int64_t m = sep.s->len;
  for (;;) {
    const char* q = p;
    while (q + m <= end && (m == 0 || memcmp(q, sep.s->s, m) != 0)) q++;
    if (n == cap) { cap *= 2; V* np = alloc(cap * sizeof(V)); memcpy(np, parts, n * sizeof(V)); parts = np; }
    if (q + m > end || m == 0) { parts[n++] = mk_str(p, end - p); break; }
    parts[n++] = mk_str(p, q - p); p = q + m;
  }
  return mk_arr(n, parts);
}
static V h_slice(V s, V i, V j) {
  int64_t len = utf8_len(s.s->s, s.s->len);
  if (!(0 <= i.i && i.i <= j.i && j.i <= len)) {
    char m[96]; snprintf(m, sizeof m, "slice の範囲 %lld..%lld が文字列の外", (long long)i.i, (long long)j.i); die(m);
  }
  int64_t c = 0, a = 0, b = s.s->len;
  for (int64_t k = 0; k <= s.s->len; k++) {
    if (k == s.s->len || (s.s->s[k] & 0xC0) != 0x80) { if (c == i.i) a = k; if (c == j.i) { b = k; break; } c++; }
  }
  return mk_str(s.s->s + a, b - a);
}
static V h_field(V x, const char* f) {
  for (int k = 0; k < x.d->n; k++) if (strcmp(CTOR_FIELDS[x.d->ctor][k], f) == 0) return x.d->f[k];
  die("フィールドが無い"); return U;
}
"#;

pub fn emit(prog: &Program) -> String {
    let ctors: Vec<&crate::parser::Ctor> = prog.datas.iter().flat_map(|d| &d.ctors).collect();
    let mut g = Gen {
        fns: prog.fns.iter().map(|f| (f.name.as_str(), f)).collect(),
        ctors: ctors
            .iter()
            .enumerate()
            .map(|(i, c)| (c.name.as_str(), i))
            .collect(),
        lines: Vec::new(),
        scopes: Vec::new(),
        lambdas: Vec::new(),
        protos: Vec::new(),
        next: 0,
    };
    let mut out = String::from(PRELUDE);
    // 構築子の名前とフィールド名の表
    let names: Vec<String> = ctors.iter().map(|c| lit(&c.name)).collect();
    out += &format!(
        "static const char* CTOR_NAMES[] = {{ {} }};\n",
        if names.is_empty() {
            "0".into()
        } else {
            names.join(", ")
        }
    );
    for (i, c) in ctors.iter().enumerate() {
        let fs: Vec<String> = c.fields.iter().map(|f| lit(&f.0)).collect();
        out += &format!(
            "static const char* CF{i}[] = {{ {} }};\n",
            if fs.is_empty() {
                "0".into()
            } else {
                fs.join(", ")
            }
        );
    }
    let cf: Vec<String> = (0..ctors.len()).map(|i| format!("CF{i}")).collect();
    out += &format!(
        "static const char** CTOR_FIELDS[] = {{ {} }};\n",
        if cf.is_empty() {
            "0".into()
        } else {
            cf.join(", ")
        }
    );
    out += RUNTIME;
    let mut defs = String::new();
    for f in &prog.fns {
        defs += &g.func(f);
    }
    for p in &g.protos {
        out += p;
    }
    out += &defs;
    for l in &g.lambdas {
        out += l;
    }
    out += "int main(void) { f_main(); return 0; }\n";
    out
}

// C の文字列リテラル。ASCII 以外と制御文字は8進のエスケープにする
fn lit(s: &str) -> String {
    let mut o = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => o += "\\\"",
            b'\\' => o += "\\\\",
            b' '..=b'~' => o.push(b as char),
            b => o += &format!("\\{b:03o}"),
        }
    }
    o + "\""
}

fn str_lit(s: &str) -> String {
    format!("mk_str({}, {})", lit(s), s.len())
}

// 衛生性で付けた # などを C の名前に使える形にする
fn ident(prefix: &str, n: &str) -> String {
    format!("{prefix}{}", n.replace('#', "_h"))
}

struct Gen<'a> {
    fns: HashMap<&'a str, &'a Fn>,
    // 構築子の名前から番号
    ctors: HashMap<&'a str, usize>,
    lines: Vec<String>,
    // hibana の変数名から C の名前
    scopes: Vec<HashMap<String, String>>,
    // 無名関数の定義と、すべての関数の前方宣言
    lambdas: Vec<String>,
    protos: Vec<String>,
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

    fn bind(&mut self, n: &str) -> String {
        self.next += 1;
        let c = format!("{}_{}", ident("v_", n), self.next);
        self.scopes
            .last_mut()
            .unwrap()
            .insert(n.to_string(), c.clone());
        c
    }

    fn lookup(&self, n: &str) -> Option<&String> {
        self.scopes.iter().rev().find_map(|s| s.get(n))
    }

    fn let_(&mut self, v: String) -> String {
        let t = self.tmp();
        self.push(format!("V {t} = {v};"));
        t
    }

    fn func(&mut self, f: &Fn) -> String {
        let name = ident("f_", &f.name);
        let ps: Vec<String> = f.params.iter().map(|_| "V".to_string()).collect();
        self.protos.push(format!(
            "static V {name}({});\nstatic V {}(V* env, V* a);\n",
            ps.join(", "),
            ident("w_", &f.name)
        ));
        self.lines.clear();
        self.scopes = vec![HashMap::new()];
        let params: Vec<String> = f
            .params
            .iter()
            .map(|(p, _)| format!("V {}", self.bind(p)))
            .collect();
        let r = self.block(&f.body);
        self.push(format!("return {r};"));
        let body = std::mem::take(&mut self.lines).join("\n");
        // 関数を値として使う時の入口
        let args: Vec<String> = (0..f.params.len()).map(|i| format!("a[{i}]")).collect();
        format!(
            "static V {name}({}) {{\n{body}\n}}\nstatic V {}(V* env, V* a) {{ (void)env; (void)a; return {name}({}); }}\n",
            params.join(", "),
            ident("w_", &f.name),
            args.join(", ")
        )
    }

    fn block(&mut self, b: &Block) -> String {
        self.scopes.push(HashMap::new());
        for s in &b.stmts {
            match s {
                Stmt::Let { name, init, .. } => {
                    let v = self.expr(init);
                    let c = self.bind(name);
                    self.push(format!("V {c} = {v};"));
                }
                Stmt::Expr(e) => {
                    self.expr(e);
                }
            }
        }
        let r = match &b.tail {
            Some(e) => self.expr(e),
            None => "U".into(),
        };
        self.scopes.pop();
        r
    }

    fn branch(&mut self, t: &str, f: impl FnOnce(&mut Self) -> String) {
        self.push("{".into());
        let v = f(self);
        self.push(format!("{t} = {v};"));
        self.push("}".into());
    }

    fn args(&mut self, args: &[Expr]) -> Vec<String> {
        args.iter().map(|a| self.expr(a)).collect()
    }

    fn array(vs: &[String]) -> String {
        if vs.is_empty() {
            "NULL".into()
        } else {
            format!("(V[]){{ {} }}", vs.join(", "))
        }
    }

    fn expr(&mut self, e: &Expr) -> String {
        match e {
            Expr::At(_, x) => self.expr(x),
            Expr::Int(n) => format!("mk_int({n}LL)"),
            Expr::Bool(b) => format!("mk_bool({})", *b as u8),
            Expr::Str(s) => self.let_(str_lit(s)),
            Expr::Var(n) => match self.lookup(n) {
                Some(c) => c.clone(),
                None => match (self.ctors.get(n.as_str()), self.fns.get(n.as_str())) {
                    (Some(i), _) => self.let_(format!("mk_data({i}, 0, NULL)")),
                    (_, Some(f)) if f.alias && f.params.is_empty() => {
                        self.let_(format!("{}()", ident("f_", n)))
                    }
                    (_, Some(_)) => self.let_(format!("mk_fn({}, 0, NULL)", ident("w_", n))),
                    // 型は名前だけ
                    _ => format!("mk_ty({})", lit(n)),
                },
            },
            Expr::Unary(op, x) => {
                let a = self.expr(x);
                self.let_(if *op == "-" {
                    format!("h_neg({a})")
                } else {
                    format!("mk_bool(!{a}.b)")
                })
            }
            Expr::Binary(op @ ("&&" | "||"), l, r) => {
                let a = self.expr(l);
                let t = self.tmp();
                self.push(format!("V {t} = {a};"));
                self.push(format!("if ({}{t}.b)", if *op == "&&" { "" } else { "!" }));
                self.branch(&t, |g| g.expr(r));
                t
            }
            Expr::Binary(op, l, r) => {
                let (a, b) = (self.expr(l), self.expr(r));
                let v = match *op {
                    "+" => format!("h_add({a}, {b})"),
                    "-" => format!("h_sub({a}, {b})"),
                    "*" => format!("h_mul({a}, {b})"),
                    "/" => format!("h_div({a}, {b})"),
                    "%" => format!("h_rem({a}, {b})"),
                    "==" => format!("h_eq({a}, {b})"),
                    "!=" => format!("mk_bool(!eq({a}, {b}))"),
                    op => format!("mk_bool({a}.i {op} {b}.i)"),
                };
                self.let_(v)
            }
            Expr::Call(name, args) => {
                if let Some(c) = self.lookup(name).cloned() {
                    let vs = self.args(args);
                    return self.let_(format!("h_call({c}, {})", Self::array(&vs)));
                }
                let mut vs = self.args(args);
                let v = match name.as_str() {
                    "print" => format!("h_print({})", vs[0]),
                    "to_str" => format!("h_str({})", vs[0]),
                    "len" | "split" | "starts_with" | "slice" => {
                        format!("h_{name}({})", vs.join(", "))
                    }
                    n => match (self.ctors.get(n), self.fns.get(n)) {
                        (Some(i), _) => format!("mk_data({i}, {}, {})", vs.len(), Self::array(&vs)),
                        (_, Some(f)) => {
                            // 推論したジェネリック引数は実行時には要らないので unit で埋める
                            if vs.len() + f.implicit == f.params.len() {
                                let mut full = vec!["U".to_string(); f.implicit];
                                full.append(&mut vs);
                                vs = full;
                            }
                            format!("{}({})", ident("f_", n), vs.join(", "))
                        }
                        // 型を作る呼び出し（Vec<int> など）。実行時には名前だけ
                        _ => format!("mk_ty({})", lit(n)),
                    },
                };
                self.let_(v)
            }
            Expr::Apply(f, args) => {
                let f = self.expr(f);
                let vs = self.args(args);
                self.let_(format!("h_call({f}, {})", Self::array(&vs)))
            }
            Expr::Field(x, f) => {
                let a = self.expr(x);
                self.let_(format!("h_field({a}, {})", lit(f)))
            }
            Expr::Index(v, i) => {
                let (a, b) = (self.expr(v), self.expr(i));
                // 範囲は型検査で証明済み
                self.let_(format!("{a}.a->items[{b}.i]"))
            }
            Expr::Array(items) => {
                let vs = self.args(items);
                self.let_(format!("mk_arr({}, {})", vs.len(), Self::array(&vs)))
            }
            Expr::Assign(n, x) => {
                let v = self.expr(x);
                let c = self.lookup(n).expect("型検査済み").clone();
                self.push(format!("{c} = {v};"));
                "U".into()
            }
            Expr::If(c, then, els) => {
                let c = self.expr(c);
                let t = self.tmp();
                self.push(format!("V {t} = U;"));
                self.push(format!("if ({c}.b)"));
                self.branch(&t, |g| g.block(then));
                if let Some(els) = els {
                    self.push("else".into());
                    self.branch(&t, |g| g.expr(els));
                }
                t
            }
            Expr::While(c, body) => {
                self.push("for (;;) {".into());
                let c = self.expr(c);
                self.push(format!("if (!{c}.b) break;"));
                self.block(body);
                self.push("}".into());
                "U".into()
            }
            Expr::For(x, it, end, body) => {
                let a = self.expr(it);
                let i = self.tmp();
                match end {
                    Some(end) => {
                        let b = self.expr(end);
                        self.push(format!("for (int64_t {i} = {a}.i; {i} < {b}.i; {i}++) {{"));
                        self.scopes.push(HashMap::new());
                        let c = self.bind(x);
                        self.push(format!("V {c} = mk_int({i});"));
                    }
                    None => {
                        self.push(format!("for (int64_t {i} = 0; {i} < {a}.a->n; {i}++) {{"));
                        self.scopes.push(HashMap::new());
                        let c = self.bind(x);
                        self.push(format!("V {c} = {a}.a->items[{i}];"));
                    }
                }
                self.block(body);
                self.scopes.pop();
                self.push("}".into());
                "U".into()
            }
            Expr::Match(s, arms) => self.match_(s, arms),
            Expr::Block(b) => {
                let t = self.tmp();
                self.push(format!("V {t} = U;"));
                self.branch(&t, |g| g.block(b));
                t
            }
            Expr::Return(x) => {
                let v = match x {
                    Some(x) => self.expr(x),
                    None => "U".into(),
                };
                self.push(format!("return {v};"));
                "U".into()
            }
            Expr::Lam(params, _, body) => self.lambda(params, body),
            Expr::Pi(..) => "mk_ty(\"fn\")".into(),
            Expr::Refine(..) => "mk_ty(\"refine\")".into(),
            Expr::Macro(..) | Expr::Quote(_) | Expr::Method(..) => unreachable!("型検査で弾く"),
        }
    }

    // 無名関数。見えている局所変数をすべて写し取り、別の C の関数にする
    fn lambda(&mut self, params: &[(String, Option<Expr>)], body: &Block) -> String {
        let mut seen = Vec::new();
        let mut caps: Vec<(String, String)> = Vec::new();
        for s in self.scopes.iter().rev() {
            for (n, c) in s {
                if !seen.contains(n) {
                    seen.push(n.clone());
                    caps.push((n.clone(), c.clone()));
                }
            }
        }
        self.next += 1;
        let name = format!("l_{}", self.next);
        let saved_lines = std::mem::take(&mut self.lines);
        let saved_scopes = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
        for (i, (n, _)) in caps.iter().enumerate() {
            let c = self.bind(n);
            self.push(format!("V {c} = env[{i}];"));
        }
        for (i, (p, _)) in params.iter().enumerate() {
            let c = self.bind(p);
            self.push(format!("V {c} = a[{i}];"));
        }
        let r = self.block(body);
        self.push(format!("return {r};"));
        let lines = std::mem::replace(&mut self.lines, saved_lines);
        self.scopes = saved_scopes;
        self.protos
            .push(format!("static V {name}(V* env, V* a);\n"));
        self.lambdas.push(format!(
            "static V {name}(V* env, V* a) {{\n(void)env; (void)a;\n{}\n}}\n",
            lines.join("\n")
        ));
        let env: Vec<String> = caps.into_iter().map(|(_, c)| c).collect();
        self.let_(format!(
            "mk_fn({name}, {}, {})",
            env.len(),
            Self::array(&env)
        ))
    }

    fn match_(&mut self, s: &Expr, arms: &[Arm]) -> String {
        let v = self.expr(s);
        let t = self.tmp();
        self.push(format!("V {t} = U;"));
        for (k, arm) in arms.iter().enumerate() {
            let head = if arm.ctor == "_" {
                "{".to_string()
            } else {
                format!("if ({v}.d->ctor == {}) {{", self.ctors[arm.ctor.as_str()])
            };
            self.push(if k == 0 { head } else { format!("else {head}") });
            self.scopes.push(HashMap::new());
            for (i, b) in arm.binds.iter().enumerate() {
                if b != "_" {
                    let c = self.bind(b);
                    self.push(format!("V {c} = {v}.d->f[{i}];"));
                }
            }
            let r = self.expr(&arm.body);
            self.scopes.pop();
            self.push(format!("{t} = {r};"));
            self.push("}".into());
        }
        t
    }
}
