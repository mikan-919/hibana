// モジュール。use をたどってファイルを読み、名前をモジュールで修飾して一つのプログラムにする
// ponytail: use は "パス".* と std.* と "パス.ts" だけ。名前の表は線形探索。循環した use は検査しない
use "./io.ts"
use "./lexer.hb".*
use "./parser.hb".*
use std.*

// 修飾しない名前と修飾した名前
struct Name { short: str, full: str }
struct Module { path: str, names: Vec<Name>, info: Ext }
struct L { done: Ref<Vec<Module>>, out: Ref<Vec<Program>> }

fn dir_of(path: str) -> str {
    let ps = split(path, "/")
    let mut s = ""
    for i in 0..len(ps) - 1 { s = s + (if i > 0 { "/" } else { "" }) + ps[i] }
    s
}

fn ends_with(s: str, t: str) -> bool { len(s) >= len(t) && slice(s, len(s) - len(t), len(s)) == t }

fn base_name(path: str) -> str {
    let mut b = ""
    for x in split(path, "/") { b = x }
    let mut n = ""
    for x in split(b, ".") { if n == "" { n = x } }
    n
}

fn spec_path(from: str, spec: str) -> str {
    if spec == "std" { io.stdPath() } else if starts_with(spec, "./") { dir_of(from) + "/" + slice(spec, 2, len(spec)) } else { dir_of(from) + "/" + spec }
}

fn find_module(l: L, path: str) -> Option<Module> {
    let mut r: Option<Module> = None
    for m in read(l.done) { if m.path == path { r = Some(m) } }
    r
}

fn full_name(names: Vec<Name>, n: str) -> str {
    let mut r = ""
    for x in names { if r == "" && x.short == n { r = x.full } }
    if r == "" { n } else { r }
}

// ---- TypeScript の関数の宣言 ----

// i 番目のトークン。範囲の外は TEof
fn tk(ts: Vec<Token>, i: int) -> Tok { if i >= 0 && i < len(ts) { ts[i].tok } else { TEof } }

fn ts_ty(ts: Vec<Token>, i: int) -> Expr {
    let n = match tk(ts, i) { TIdent(n) => n, _ => "" }
    let base = if n == "string" { "str" } else if n == "number" { "int" } else if n == "boolean" { "bool" } else if n == "void" { "unit" } else { panic("読めない TypeScript の型 " + n) }
    if is_p(tk(ts, i + 1), "[") && is_p(tk(ts, i + 2), "]") { ECall("Vec", [EVar(base)]) } else { EVar(base) }
}

// export function 名前(引数: 型, ...): 型 を、別名.名前 という関数にする
// ponytail: number は BigInt に変換しない。型は string・number・boolean・void とその配列だけ
fn ts_fns(src: str, alias: str, path: str) -> Vec<Func> {
    let ts = match lex(src) { Ok(ts) => ts, Err(e) => panic(path + ": " + e.msg) }
    let mut fs: Vec<Func> = []
    for i in 0..len(ts) {
        if tk(ts, i) == TIdent("export") && tk(ts, i + 1) == TIdent("function") {
            let name = match tk(ts, i + 2) { TIdent(n) => n, _ => "" }
            let mut k = i + 4
            let mut ps: Vec<Expr> = []
            while k < len(ts) && !is_p(tk(ts, k), ")") {
                let pn = match tk(ts, k) { TIdent(n) => n, _ => "" }
                ps = push(ps, EParam(pn, Some(ts_ty(ts, k + 2))))
                k = k + 3
                while k < len(ts) && !is_p(tk(ts, k), ",") && !is_p(tk(ts, k), ")") { k = k + 1 }
                if is_p(tk(ts, k), ",") { k = k + 1 }
            }
            let ret = if is_p(tk(ts, k + 1), ":") { Some(ts_ty(ts, k + 2)) } else { None }
            fs = push(fs, Func { name: alias + "." + name, implicit: 0, is_partial: true, params: ps, ret: ret, effs: [], body: EBlock([], None), kind: "js:" + path })
        }
    }
    fs
}

// ---- 名前の解決 ----

// R は解決に使う名前の表と、JS のモジュールの別名
struct R { names: Vec<Name>, js: Vec<str> }

partial fn rs(r: R, env: Vec<str>, xs: Vec<Expr>) -> Vec<Expr> { map(xs, fn(x: Expr) -> Expr { re(r, env, x) }) }

partial fn ro(r: R, env: Vec<str>, o: Option<Expr>) -> Option<Expr> { match o { Some(x) => Some(re(r, env, x)), None => None } }

fn name_of(r: R, env: Vec<str>, n: str) -> str { if contains(env, n) { n } else { full_name(r.names, n) } }

// 引数の並び。前の引数の名前は後の引数の型から見える
partial fn rparams(r: R, env0: Vec<str>, ps: Vec<Expr>) -> Vec<Expr> {
    let mut env = env0
    let mut out: Vec<Expr> = []
    for p in ps {
        match p {
            EParam(n, t) => { out = push(out, EParam(n, ro(r, env, t))); env = push(env, n) }
            _ => { out = push(out, re(r, env, p)) }
        }
    }
    out
}

fn param_names(ps: Vec<Expr>) -> Vec<str> { map(ps, fn(p: Expr) -> str { match p { EParam(n, t) => n, _ => "" } }) }

partial fn re(r: R, env: Vec<str>, e: Expr) -> Expr {
    match e {
        EVar(n) => EVar(name_of(r, env, n)),
        EUnary(op, x) => EUnary(op, re(r, env, x)),
        EBinary(op, a, b) => EBinary(op, re(r, env, a), re(r, env, b)),
        ECall(f, xs) => ECall(name_of(r, env, f), rs(r, env, xs)),
        EField(x, f) => EField(re(r, env, x), f),
        EMethod(x, f, xs) => match x {
            // 別名.関数(引数) は JS のモジュールの関数の呼び出し
            EVar(m) => if !contains(env, m) && contains(r.js, m) { ECall(m + "." + f, rs(r, env, xs)) } else { EMethod(re(r, env, x), f, rs(r, env, xs)) },
            _ => EMethod(re(r, env, x), f, rs(r, env, xs)),
        },
        EAssign(n, x) => EAssign(n, re(r, env, x)),
        EIf(c, t, els) => EIf(re(r, env, c), re(r, env, t), ro(r, env, els)),
        EWhile(c, b) => EWhile(re(r, env, c), re(r, env, b)),
        EBlock(stmts, tail) => {
            let mut env2 = env
            let mut out: Vec<Expr> = []
            for s in stmts {
                match s {
                    ELet(n, m, t, init) => { out = push(out, ELet(n, m, ro(r, env2, t), re(r, env2, init))); env2 = push(env2, n) }
                    _ => { out = push(out, re(r, env2, s)) }
                }
            }
            EBlock(out, ro(r, env2, tail))
        }
        EReturn(x) => EReturn(ro(r, env, x)),
        EArray(xs) => EArray(rs(r, env, xs)),
        EIndex(x, i) => EIndex(re(r, env, x), re(r, env, i)),
        ERefine(t, x, c) => ERefine(re(r, env, t), x, re(r, push(env, x), c)),
        EMatch(x, arms) => EMatch(re(r, env, x), rs(r, env, arms)),
        EArm(c, bs, body) => {
            let mut env2 = env
            for b in bs { env2 = push(env2, b) }
            EArm(if c == "_" { c } else { full_name(r.names, c) }, bs, re(r, env2, body))
        }
        ELam(ps, ret, effs, body) => {
            let env2 = concat_str(env, param_names(ps))
            ELam(rparams(r, env, ps), ro(r, env2, ret), rs(r, env2, effs), re(r, env2, body))
        }
        EPi(ps, ret, effs) => {
            let env2 = concat_str(env, param_names(ps))
            EPi(rparams(r, env, ps), re(r, env2, ret), rs(r, env2, effs))
        }
        EApply(f, xs) => EApply(re(r, env, f), rs(r, env, xs)),
        EFor(x, a, b, body) => EFor(x, re(r, env, a), ro(r, env, b), re(r, push(env, x), body)),
        EParam(n, t) => EParam(n, ro(r, env, t)),
        EHandle(x, arms) => EHandle(re(r, env, x), rs(r, env, arms)),
        _ => e,
    }
}

partial fn resolve_fn(r: R, f: Func) -> Func {
    let name = if f.kind == "impl" {
        let parts = split(f.name, "#")
        if len(parts) == 2 { full_name(r.names, parts[0]) + "#" + full_name(r.names, parts[1]) } else { f.name }
    } else if f.kind == "default" { full_name(r.names, slice(f.name, 0, len(f.name) - 8)) + "#default" }
    else if starts_with(f.kind, "js:") { f.name } else { full_name(r.names, f.name) }
    let env = param_names(f.params)
    Func { name: name, implicit: f.implicit, is_partial: f.is_partial, params: rparams(r, [], f.params), ret: ro(r, env, f.ret), effs: rs(r, env, f.effs), body: re(r, env, f.body), kind: f.kind }
}

partial fn resolve_data(r: R, d: Data) -> Data {
    let env = param_names(d.params)
    let cs = map(d.ctors, fn(c: Ctor) -> Ctor { Ctor { name: full_name(r.names, c.name), fields: rparams(r, env, c.fields), ret: ro(r, env, c.ret) } })
    Data { name: full_name(r.names, d.name), params: rparams(r, [], d.params), ctors: cs }
}

// モジュールが宣言した名前。prefix が "" なら修飾しない
fn own_names(prog: Program, prefix: str) -> Vec<Name> {
    let q = fn(n: str) -> Name { Name { short: n, full: if prefix == "" { n } else { prefix + "." + n } } }
    let mut ns: Vec<Name> = []
    for d in prog.datas {
        ns = push(ns, q(d.name))
        for c in d.ctors { ns = push(ns, q(c.name)) }
    }
    for f in prog.fns { if f.kind != "impl" && f.kind != "default" { ns = push(ns, q(f.name)) } }
    // トレイトの名前は型の位置にしか現れず、検査も生成も使わないので修飾しない
    ns
}

fn concat_names(a: Vec<Name>, b: Vec<Name>) -> Vec<Name> {
    let mut r = a
    for x in b { r = push(r, x) }
    r
}

// path のモジュールと、それが use するモジュールを読む。root は利用者のファイル（名前を修飾しない）
partial fn load_module(l: L, path: str, root: bool) {
    if is_some_m(find_module(l, path)) { return }
    let src = io.readFile(path)
    let specs = match lex(src) { Ok(ts) => uses_of(ts), Err(e) => panic(path + ": " + e.msg) }
    let mut deps: Vec<Module> = []
    let mut js: Vec<str> = []
    let mut js_fns: Vec<Func> = []
    for s in specs {
        let p = spec_path(path, s)
        if ends_with(s, ".ts") {
            let alias = base_name(p)
            js = push(js, alias)
            js_fns = concat_f(js_fns, ts_fns(io.readFile(p), alias, p))
        } else {
            load_module(l, p, false)
            match find_module(l, p) { Some(m) => { deps = push(deps, m) }, None => {} }
        }
    }
    let mut ext = no_ext()
    for m in deps { ext = Ext { types: concat_str(ext.types, m.info.types), generics: concat_str(ext.generics, m.info.generics), structs: concat_si(ext.structs, m.info.structs) } }
    let prog = parse_with(src, ext)
    let own = own_names(prog, if root { "" } else { base_name(path) })
    let mut names = own
    for m in deps { names = concat_names(names, m.names) }
    let r = R { names: names, js: js }
    let out = Program { datas: map(prog.datas, fn(d: Data) -> Data { resolve_data(r, d) }), fns: concat_f(js_fns, map(prog.fns, fn(f: Func) -> Func { resolve_fn(r, f) })), uses: [], info: prog.info }
    write(l.done, push(read(l.done), Module { path: path, names: own, info: prog.info }))
    write(l.out, push(read(l.out), out))
}

fn is_some_m(o: Option<Module>) -> bool { match o { Some(m) => true, None => false } }

fn concat_si(a: Vec<StructInfo>, b: Vec<StructInfo>) -> Vec<StructInfo> {
    let mut r = a
    for x in b { r = push(r, x) }
    r
}

// path のファイルと、それが use するすべてのモジュールを一つのプログラムにする
partial fn load(path: str) -> Program {
    let l = L { done: new_ref([]), out: new_ref([]) }
    load_module(l, io.resolve(path), true)
    let mut datas: Vec<Data> = []
    let mut fns: Vec<Func> = []
    // 複数のモジュールが use した JS のモジュールの関数は一つにする
    let mut js: Vec<str> = []
    for p in read(l.out) {
        for d in p.datas { datas = push(datas, d) }
        for f in p.fns {
            if !starts_with(f.kind, "js:") || !contains(js, f.name) { fns = push(fns, f) }
            if starts_with(f.kind, "js:") { js = push(js, f.name) }
        }
    }
    Program { datas: datas, fns: fns, uses: [], info: no_ext() }
}
