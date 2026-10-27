// コード生成。selfhost/parser.hb の構文木から JavaScript を作る（src/js.rs と同じ値の表し方）
// ponytail: 中核の構文だけ。push は毎回写す（その場で足す最適化は無い）。効果・JS の import は後で足す
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
                spill(g, Code { pre: a.pre, val: "f_" + name + "(" + a.val + ")" })
            }
            None => {
                let a = args(g, env, xs)
                if is_ctor(g, name) { spill(g, Code { pre: a.pre, val: "{ $c: " + quote_str(name) + ", $f: [" + a.val + "] }" }) }
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
    else { "$" + name + "(" + a + ")" }
}

partial fn gen(g: G, env: Vec<Var>, e: Expr) -> Code {
    match e {
        EInt(n) => pure(to_str(n) + "n"),
        EBool(b) => pure(to_str(b)),
        EStr(s) => pure(quote_str(s)),
        EVar(n) => match lookup_var(env, n) {
            Some(v) => pure(v.js),
            None => if is_ctor(g, n) { pure("{ $c: " + quote_str(n) + ", $f: [] }") } else { pure("f_" + n) },
        },
        EUnary(op, x) => { let c = gen(g, env, x); Code { pre: c.pre, val: "(" + op + c.val + ")" } }
        EBinary(op, a, b) => binary(g, env, op, a, b),
        ECall(f, xs) => call(g, env, f, xs),
        EApply(f, xs) => {
            let fc = spill(g, gen(g, env, f))
            let a = args(g, env, xs)
            spill(g, Code { pre: fc.pre + a.pre, val: fc.val + "(" + a.val + ")" })
        }
        EField(x, f) => { let c = gen(g, env, x); Code { pre: c.pre, val: "$get(" + c.val + ", " + quote_str(f) + ")" } }
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
                let test = if ctor == "_" { "true" } else { s.val + ".$c === " + quote_str(ctor) }
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
    "function $str(v) { return typeof v === \"string\" ? v : $show(v); }\n" +
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
    "function $panic(m) { throw new Error(m); }\n"
}

// プログラム全体の JavaScript。最後に main を呼ぶ
partial fn compile(prog: Program) -> str {
    let g = G { prog: prog, n: new_ref(0) }
    let mut out = runtime()
    let mut fields: Vec<str> = []
    for d in prog.datas {
        for c in d.ctors {
            fields = push(fields, quote_str(c.name) + ": [" + join(map(c.fields, fn(p: Expr) -> str { quote_str(param_name(p)) }), ", ") + "]")
        }
    }
    out = out + "const $fields = {" + join(fields, ", ") + "};\n"
    for f in prog.fns {
        if !is_alias(f) {
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
            out = out + "function f_" + f.name + "(" + join(names, ", ") + ") {\n" + b.pre + "return " + b.val + ";\n}\n"
        }
    }
    out + "f_main();\n"
}
