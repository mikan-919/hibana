// コード生成。selfhost/parser.hb の構文木から JavaScript を作る（src/js.rs と同じ値の表し方）
// ponytail: push は毎回写す（その場で足す最適化は無い）。Async は意味を持たず、all は順に実行する
// 効果は src/js.rs と同じ形: 処理中の handle の並び $h から腕を探し、resume しない腕の値は例外 $Abort で handle まで戻す
// トレイトのメソッドは、最初の引数の実行時の型で実装を選ぶ（型引数が戻り値にだけ現れるメソッドは呼べない）
use "./parser.hb".*
use std.*

// 式の JavaScript。pre は先に実行する文、val は値の式（副作用の無い式か一時変数）
struct Code { pre: str, val: str }
// 引数の並び。vals は左から順の値
struct Codes { pre: str, vals: Vec<str> }
struct Var { name: str, js: str, mutable: bool }
struct G { prog: Program, n: Ref<int> }

fn tmp(g: G) -> str {
    write(g.n, read(g.n) + 1)
    "t" + to_str(read(g.n))
}

fn pure(v: str) -> Code { Code { pre: "", val: v } }

// 値を一時変数に入れる
fn spill(g: G, c: Code) -> Code {
    let t = tmp(g)
    Code { pre: c.pre + "const " + t + " = " + c.val + ";\n", val: t }
}

fn lookup_var(env: Vec<Var>, name: str) -> Option<Var> {
    let mut r: Option<Var> = None
    for v in env { if v.name == name { r = Some(v) } }
    r
}

fn find_fn(g: G, name: str) -> Option<Func> {
    let mut r: Option<Func> = None
    for f in g.prog.fns { if f.name == name { r = Some(f) } }
    r
}

fn is_ctor(g: G, name: str) -> bool {
    let mut r = false
    for d in g.prog.datas { for c in d.ctors { if c.name == name { r = true } } }
    r
}

fn join(xs: Vec<str>, sep: str) -> str {
    let mut s = ""
    for i in 0..len(xs) { s = s + (if i > 0 { sep } else { "" }) + xs[i] }
    s
}

// 引数を左から順に評価する。後の引数に文があれば、前の値を一時変数に入れて順序を保つ
partial fn args(g: G, env: Vec<Var>, xs: Vec<Expr>) -> Code {
    let a = arg_list(g, env, xs)
    Code { pre: a.pre, val: join(a.vals, ", ") }
}

partial fn arg_list(g: G, env: Vec<Var>, xs: Vec<Expr>) -> Codes {
    let mut pre = ""
    let mut vals: Vec<str> = []
    for x in xs {
        let c = gen(g, env, x)
        if c.pre != "" && len(vals) > 0 {
            let mut spilled: Vec<str> = []
            for v in vals { let t = tmp(g); pre = pre + "const " + t + " = " + v + ";\n"; spilled = push(spilled, t) }
            vals = spilled
        }
        pre = pre + c.pre
        vals = push(vals, c.val)
    }
    Codes { pre: pre, vals: vals }
}

partial fn call(g: G, env: Vec<Var>, name: str, xs: Vec<Expr>) -> Code {
    match lookup_var(env, name) {
        Some(v) => { let a = args(g, env, xs); spill(g, Code { pre: a.pre, val: v.js + "(" + a.val + ")" }) }
        None => match find_fn(g, name) {
            Some(f) => {
                // f<型, ...>(引数) の型引数は実行時には要らない
                let mut rest: Vec<Expr> = []
                for i in 0..len(xs) { if !(f.implicit > 0 && len(xs) == len(f.params) && i < f.implicit) { rest = push(rest, xs[i]) } }
                let a = args(g, env, rest)
                spill(g, Code { pre: a.pre, val: fname(name) + "(" + a.val + ")" })
            }
            None => {
                let a = args(g, env, xs)
                if is_ctor(g, name) { spill(g, Code { pre: a.pre, val: "{ $c: " + dquote(name) + ", $f: [" + a.val + "] }" }) }
                else { spill(g, Code { pre: a.pre, val: builtin(name, a.val) }) }
            }
        },
    }
}

fn builtin(name: str, a: str) -> str {
    if name == "print" { "$print(" + a + ")" }
    else if name == "to_str" { "$str(" + a + ")" }
    else if name == "push" { "$push(" + a + ")" }
    else if name == "new_ref" { "({ v: " + a + " })" }
    else if name == "read" { "(" + a + ").v" }
    else if name == "write" { "$write(" + a + ")" }
    else if name == "resume" { "{ $c: \"$resume\", $f: [" + a + "] }" }
    else if name == "all" { "(" + a + ").map((f) => f())" }
    else { "$" + name + "(" + a + ")" }
}

partial fn gen(g: G, env: Vec<Var>, e: Expr) -> Code {
    match e {
        EInt(n) => pure(to_str(n) + "n"),
        EBool(b) => pure(to_str(b)),
        EStr(s) => pure(dquote(s)),
        EVar(n) => match lookup_var(env, n) {
            Some(v) => pure(v.js),
            None => if is_ctor(g, n) { pure("{ $c: " + dquote(n) + ", $f: [] }") } else { pure(fname(n)) },
        },
        EUnary(op, x) => { let c = gen(g, env, x); Code { pre: c.pre, val: "(" + op + c.val + ")" } }
        EBinary(op, a, b) => binary(g, env, op, a, b),
        ECall(f, xs) => call(g, env, f, xs),
        EApply(f, xs) => {
            let fc = spill(g, gen(g, env, f))
            let a = args(g, env, xs)
            spill(g, Code { pre: fc.pre + a.pre, val: fc.val + "(" + a.val + ")" })
        }
        EField(x, f) => { let c = gen(g, env, x); Code { pre: c.pre, val: "$get(" + c.val + ", " + dquote(f) + ")" } }
        EAssign(n, x) => {
            let c = gen(g, env, x)
            let js = match lookup_var(env, n) { Some(v) => v.js, None => n }
            Code { pre: c.pre + js + " = " + c.val + ";\n", val: "undefined" }
        }
        EIf(cond, t, els) => {
            let c = gen(g, env, cond)
            let r = tmp(g)
            let tc = gen(g, env, t)
            let ec = match els { Some(x) => gen(g, env, x), None => pure("undefined") }
            Code { pre: c.pre + "let " + r + ";\nif (" + c.val + ") {\n" + tc.pre + r + " = " + tc.val + ";\n} else {\n" + ec.pre + r + " = " + ec.val + ";\n}\n", val: r }
        }
        EWhile(cond, body) => {
            let c = gen(g, env, cond)
            let b = gen(g, env, body)
            Code { pre: "while (true) {\n" + c.pre + "if (!(" + c.val + ")) break;\n" + b.pre + "}\n", val: "undefined" }
        }
        EBlock(stmts, tail) => block(g, env, stmts, tail),
        EReturn(x) => {
            let c = match x { Some(v) => gen(g, env, v), None => pure("undefined") }
            Code { pre: c.pre + "return " + c.val + ";\n", val: "undefined" }
        }
        EArray(xs) => { let a = args(g, env, xs); Code { pre: a.pre, val: "[" + a.val + "]" } }
        EIndex(v, i) => {
            let a = args(g, env, [v, i])
            let r = tmp(g)
            Code { pre: a.pre + "const " + r + " = $index(" + a.val + ");\n", val: r }
        }
        EMatch(x, arms) => match_code(g, env, x, arms),
        EHandle(x, arms) => {
            // 腕は無名関数にする。本体はその場に書き、resume しなかった腕の値を例外で受け取る
            let mut pre = ""
            let mut fs: Vec<str> = []
            for arm in arms {
                match arm {
                    EArm(op, binds, body) => {
                        let lam = gen(g, env, ELam(map(binds, fn(b: str) -> Expr { EParam(b, None) }), None, [], EBlock([], Some(body))))
                        pre = pre + lam.pre
                        fs = push(fs, dquote(op) + ": " + lam.val)
                    }
                    _ => {}
                }
            }
            let h = tmp(g)
            let r = tmp(g)
            let saved = tmp(g)
            let b = gen(g, env, x)
            Code { pre: pre + "const " + h + " = { arms: { " + join(fs, ", ") + " } };\nlet " + r + ";\nconst " + saved + " = $h; $h = [...$h, " + h + "];\ntry {\n" + b.pre + r + " = " + b.val + ";\n} catch (e) { if (e instanceof $Abort && e.h === " + h + ") " + r + " = e.v; else throw e; } finally { $h = " + saved + "; }\n", val: r }
        }
        ELam(ps, ret, effs, body) => {
            // 作った時の可変な変数の値を写し取る（インタプリタと同じ意味にする）
            let muts = map(filter(env, fn(v: Var) -> bool { v.mutable }), fn(v: Var) -> str { v.js })
            let mut env2 = env
            let mut names: Vec<str> = []
            for p in ps {
                let js = tmp(g)
                names = push(names, js)
                env2 = push(env2, Var { name: param_name(p), js: js, mutable: false })
            }
            let b = gen(g, env2, body)
            let f = "(" + join(names, ", ") + ") => {\n" + b.pre + "return " + b.val + ";\n}"
            spill(g, pure("((" + join(muts, ", ") + ") => " + f + ")(" + join(muts, ", ") + ")"))
        }
        EFor(x, a, b, body) => {
            let js = tmp(g)
            let bc = gen(g, push(env, Var { name: x, js: js, mutable: false }), body)
            match b {
                Some(hi) => {
                    let r = args(g, env, [a, hi])
                    let lo = tmp(g)
                    let up = tmp(g)
                    Code { pre: r.pre + "const [" + lo + ", " + up + "] = [" + r.val + "];\nfor (let " + js + " = " + lo + "; " + js + " < " + up + "; " + js + "++) {\n" + bc.pre + "}\n", val: "undefined" }
                }
                None => {
                    let c = gen(g, env, a)
                    Code { pre: c.pre + "for (const " + js + " of " + c.val + ") {\n" + bc.pre + "}\n", val: "undefined" }
                }
            }
        }
        _ => pure("undefined"),
    }
}

// JavaScript の関数名。修飾した名前の . や # を $ にする
fn fname(n: str) -> str {
    let mut s = "f_"
    for c in chars(n) { s = s + (if c == "." || c == "#" || c == "{" || c == "}" { "$" } else { c }) }
    s
}

fn param_name(e: Expr) -> str { match e { EParam(n, t) => n, _ => "" } }

partial fn binary(g: G, env: Vec<Var>, op: str, a: Expr, b: Expr) -> Code {
    if op == "&&" || op == "||" {
        // 右辺は左辺の値によっては実行しない
        let ac = gen(g, env, a)
        let bc = gen(g, env, b)
        let r = tmp(g)
        let test = if op == "&&" { r } else { "!" + r }
        Code { pre: ac.pre + "let " + r + " = " + ac.val + ";\nif (" + test + ") {\n" + bc.pre + r + " = " + bc.val + ";\n}\n", val: r }
    } else {
        let c = arg_list(g, env, [a, b])
        let l = if len(c.vals) > 0 { c.vals[0] } else { "" }
        let r = if len(c.vals) > 1 { c.vals[1] } else { "" }
        let v = if op == "==" { "$eq(" + l + ", " + r + ")" } else if op == "!=" { "!$eq(" + l + ", " + r + ")" }
            else if op == "/" { "$div(" + l + ", " + r + ")" } else if op == "%" { "$mod(" + l + ", " + r + ")" }
            else { "(" + l + " " + op + " " + r + ")" }
        Code { pre: c.pre, val: v }
    }
}

partial fn match_code(g: G, env: Vec<Var>, x: Expr, arms: Vec<Expr>) -> Code {
    let s = spill(g, gen(g, env, x))
    let r = tmp(g)
    let mut out = s.pre + "let " + r + ";\n"
    let mut first = true
    for arm in arms {
        match arm {
            EArm(ctor, binds, body) => {
                let mut env2 = env
                let mut bind_pre = ""
                for i in 0..len(binds) {
                    let js = tmp(g)
                    bind_pre = bind_pre + "const " + js + " = " + s.val + ".$f[" + to_str(i) + "];\n"
                    env2 = push(env2, Var { name: binds[i], js: js, mutable: false })
                }
                let b = gen(g, env2, body)
                let test = if ctor == "_" { "true" } else { s.val + ".$c === " + dquote(ctor) }
                out = out + (if first { "if (" } else { "else if (" }) + test + ") {\n" + bind_pre + b.pre + r + " = " + b.val + ";\n}\n"
                first = false
            }
            _ => {}
        }
    }
    Code { pre: out, val: r }
}

partial fn block(g: G, env0: Vec<Var>, stmts: Vec<Expr>, tail: Option<Expr>) -> Code {
    let mut env = env0
    let r = tmp(g)
    let mut pre = "let " + r + ";\n{\n"
    for s in stmts {
        match s {
            ELet(n, m, t, init) => {
                let c = gen(g, env, init)
                let js = tmp(g)
                pre = pre + c.pre + "let " + js + " = " + c.val + ";\n"
                env = push(env, Var { name: n, js: js, mutable: m })
            }
            _ => { pre = pre + gen(g, env, s).pre }
        }
    }
    let tc = match tail { Some(x) => gen(g, env, x), None => pure("undefined") }
    Code { pre: pre + tc.pre + r + " = " + tc.val + ";\n}\n", val: r }
}

fn is_alias(f: Func) -> bool {
    match f.ret { Some(t) => match t { EVar(n) => n == "Type", _ => false }, None => false }
}

// 実行時の補助関数（src/js.rs の前置きの一部と同じ動き）
fn runtime() -> str {
    "function $eq(a, b) {\n" +
    "  if (a === b) return true;\n" +
    "  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => $eq(x, b[i]));\n" +
    "  if (a && b && typeof a === \"object\" && typeof b === \"object\") return a.$c === b.$c && $eq(a.$f, b.$f);\n" +
    "  return false;\n}\n" +
    "function $show(v) {\n" +
    "  if (typeof v === \"bigint\" || typeof v === \"boolean\") return String(v);\n" +
    "  if (typeof v === \"string\") return JSON.stringify(v);\n" +
    "  if (v === undefined) return \"()\";\n" +
    "  if (typeof v === \"function\") return \"<fn>\";\n" +
    "  if (Array.isArray(v)) return \"[\" + v.map($show).join(\", \") + \"]\";\n" +
    "  if (v.$c.endsWith(\"{}\") && $fields[v.$c][0] === \"0\") return v.$c.slice(0, -2) + \"(\" + v.$f.map($show).join(\", \") + \")\";\n" +
    "  if (v.$c.endsWith(\"{}\")) return v.$c.slice(0, -2) + \" { \" + $fields[v.$c].map((f, i) => f + \": \" + $show(v.$f[i])).join(\", \") + \" }\";\n" +
    "  return v.$f.length === 0 ? v.$c : v.$c + \"(\" + v.$f.map($show).join(\", \") + \")\";\n}\n" +
    "function $key(v) {\n" +
    "  if (typeof v === \"bigint\") return \"int\";\n" +
    "  if (typeof v === \"string\") return \"str\";\n" +
    "  if (typeof v === \"boolean\") return \"bool\";\n" +
    "  if (Array.isArray(v)) return \"Vec\";\n" +
    "  return v && v.$c !== undefined ? $types[v.$c] : \"\";\n}\n" +
    "const $impls = {}, $defaults = {};\n" +
    "function $impl(m, k, f) { ($impls[m] ??= {})[k] = f; }\n" +
    "function $call(m, a) {\n" +
    "  const f = $impls[m]?.[$key(a[0])] ?? $defaults[m];\n" +
    "  if (!f) throw new Error(m + \" の \" + $key(a[0]) + \" の実装が無い\");\n" +
    "  return f(...a);\n}\n" +
    "// Show の実装がある型は to_str と print でそれを使う（Vec は組み込みの表示）\n" +
    "function $str(v) {\n" +
    "  if (typeof v === \"string\") return v;\n" +
    "  const f = v && v.$c !== undefined && $impls[\"std.show\"]?.[$types[v.$c]];\n" +
    "  return f ? f(v) : $show(v);\n}\n" +
    "function $print(v) { console.log($str(v)); }\n" +
    "function $len(v) { return BigInt(typeof v === \"string\" ? [...v].length : v.length); }\n" +
    "function $push(v, x) { return [...v, x]; }\n" +
    "function $write(r, x) { r.v = x; }\n" +
    "function $get(v, f) { return v.$f[$fields[v.$c].indexOf(f)]; }\n" +
    "function $index(v, i) { return v[Number(i)]; }\n" +
    "function $div(a, b) { if (b === 0n) throw new Error(\"0 で除算した\"); return a / b; }\n" +
    "function $mod(a, b) { if (b === 0n) throw new Error(\"0 で除算した\"); return a % b; }\n" +
    "function $split(s, sep) { return s.split(sep); }\n" +
    "function $starts_with(s, p) { return s.startsWith(p); }\n" +
    "function $slice(s, i, j) { return [...s].slice(Number(i), Number(j)).join(\"\"); }\n" +
    "function $chars(s) { return [...s]; }\n" +
    "function $code(s) { return s.length ? BigInt(s.codePointAt(0)) : -1n; }\n" +
    "function $panic(m) { throw new Error(m); }\n" +
    "let $h = [];\n" +
    "class $Abort { constructor(h, v) { this.h = h; this.v = v; } }\n" +
    "function $op(op, args) {\n" +
    "  for (let i = $h.length - 1; i >= 0; i--) {\n" +
    "    const h = $h[i];\n" +
    "    if (!Object.hasOwn(h.arms, op)) continue;\n" +
    "    const saved = $h;\n" +
    "    $h = $h.slice(0, i);\n" +
    "    let r;\n" +
    "    try { r = h.arms[op](...args); } finally { $h = saved; }\n" +
    "    if (r && r.$c === \"$resume\") return r.$f[0];\n" +
    "    throw new $Abort(h, r);\n" +
    "  }\n" +
    "  throw new Error(\"操作 \" + op + \" を処理する handle が無い\");\n}\n"
}

// プログラム全体の JavaScript。最後に main を呼ぶ
partial fn compile(prog: Program) -> str {
    let g = G { prog: prog, n: new_ref(0) }
    let mut out = runtime()
    let mut fields: Vec<str> = []
    let mut types: Vec<str> = []
    for d in prog.datas {
        for c in d.ctors {
            fields = push(fields, dquote(c.name) + ": [" + join(map(c.fields, fn(p: Expr) -> str { dquote(param_name(p)) }), ", ") + "]")
            types = push(types, dquote(c.name) + ": " + dquote(d.name))
        }
    }
    out = out + "const $fields = {" + join(fields, ", ") + "};\n"
    out = out + "const $types = {" + join(types, ", ") + "};\n"
    let mut imports = ""
    let mut paths: Vec<str> = []
    for f in prog.fns {
        if f.kind == "op" {
            out = out + "function " + fname(f.name) + "(...a) { return $op(" + dquote(f.name) + ", a); }\n"
        } else if f.kind == "trait" {
            out = out + "function " + fname(f.name) + "(...a) { return $call(" + dquote(f.name) + ", a); }\n"
        } else if starts_with(f.kind, "js:") {
            // JS のモジュールは import * as $jsK で読む
            let path = slice(f.kind, 3, len(f.kind))
            if !contains(paths, path) {
                imports = imports + "import * as $js" + to_str(len(paths)) + " from " + dquote(path) + ";\n"
                paths = push(paths, path)
            }
            let mut k = 0
            for i in 0..len(paths) { if paths[i] == path { k = i } }
            let mut js = ""
            for x in split(f.name, ".") { js = x }
            out = out + "function " + fname(f.name) + "(...a) { return $js" + to_str(k) + "." + js + "(...a); }\n"
        } else if !is_alias(f) {
            let mut env: Vec<Var> = []
            let mut names: Vec<str> = []
            for i in 0..len(f.params) {
                if i >= f.implicit {
                    let js = tmp(g)
                    names = push(names, js)
                    env = push(env, Var { name: param_name(f.params[i]), js: js, mutable: false })
                }
            }
            let b = gen(g, env, f.body)
            out = out + "function " + fname(f.name) + "(" + join(names, ", ") + ") {\n" + b.pre + "return " + b.val + ";\n}\n"
            if f.kind == "impl" {
                let parts = split(f.name, "#")
                if len(parts) == 2 { out = out + "$impl(" + dquote(parts[0]) + ", " + dquote(parts[1]) + ", " + fname(f.name) + ");\n" }
            }
            if f.kind == "default" { out = out + "$defaults[" + dquote(slice(f.name, 0, len(f.name) - 8)) + "] = " + fname(f.name) + ";\n" }
        }
    }
    imports + out + "f_main();\n"
}
