// 型検査器。selfhost/parser.hb の構文木を検査する
// ponytail: 依存型・添字付きの型族・細分化型の証明は無い（細分化型は元の型として扱う）
// 効果: 起きた効果を集め、関数の終わりで宣言と比べる。無名関数の効果は本体から推論する。Async はどこでも起こせる
// ponytail: 宣言の探索は線形。大きいプログラムで遅ければ Map にする
use "./parser.hb".*
use std.*

enum Ty {
    YInt, YBool, YStr, YUnit, YNever, YType,
    YVec(a: Ty),
    YRef(a: Ty),
    YData(name: str, args: Vec<Ty>),
    YFn(ps: Vec<Ty>, ret: Ty, effs: Vec<Ty>),
    // 効果の変数（E: Effect）の中身。効果の並び
    YRow(effs: Vec<Ty>),
    // 推論中の型変数（subst の添字）
    YVar(id: int),
    // 検査中の関数のジェネリック引数
    YParam(name: str),
}

// 型の式の中の名前と、その型
struct Bind { name: str, ty: Ty }
struct Local { name: str, ty: Ty, mutable: bool }

struct C {
    prog: Program,
    // 型変数 id の中身。決まっていなければ YVar(id) 自身
    subst: Ref<Vec<Ty>>,
    errs: Ref<Vec<str>>,
    // 検査中の関数の名前・戻り値の型・ジェネリック引数
    fname: Ref<str>,
    ret: Ref<Ty>,
    tparams: Ref<Vec<Bind>>,
    // 検査中の式で起きた効果と、handle の腕の resume に渡す型
    raised: Ref<Vec<Ty>>,
    resume: Ref<Ty>,
    // 検査中の関数が宣言した効果。起きた効果の型引数はこれから決める（need() の T など）
    declared: Ref<Vec<Ty>>,
    // 関数の終わりで impl の有無を確かめる制約
    bounds: Ref<Vec<Bound>>,
}

// ty の型にメソッドのどれかの impl が要る。call は トレイトのメソッドの呼び出し（既定の実装でもよい）
struct Bound { methods: Vec<str>, ty: Ty, call: bool }

fn err(c: C, msg: str) -> Ty {
    write(c.errs, push(read(c.errs), "関数 " + read(c.fname) + ": " + msg))
    YNever
}

// ---- 型変数 ----

fn fresh(c: C) -> Ty {
    let s = read(c.subst)
    write(c.subst, push(s, YVar(len(s))))
    YVar(len(s))
}

fn set_var(c: C, id: int, t: Ty) {
    let s = read(c.subst)
    let mut r: Vec<Ty> = []
    for i in 0..len(s) { r = push(r, if i == id { t } else { s[i] }) }
    write(c.subst, r)
}

// 決まっている型変数をたどる
partial fn walk(c: C, t: Ty) -> Ty {
    match t {
        YVar(id) => {
            let s = read(c.subst)
            if id >= 0 && id < len(s) {
                match s[id] { YVar(j) => if j == id { t } else { walk(c, s[id]) }, _ => walk(c, s[id]) }
            } else { t }
        }
        _ => t,
    }
}

partial fn show_ty(c: C, t0: Ty) -> str {
    match walk(c, t0) {
        YInt => "int", YBool => "bool", YStr => "str", YUnit => "unit", YNever => "never", YType => "Type",
        YVec(a) => "Vec<" + show_ty(c, a) + ">",
        YRef(a) => "Ref<" + show_ty(c, a) + ">",
        YData(n, args) => if len(args) == 0 { n } else { n + "<" + join(map(args, fn(a: Ty) -> str { show_ty(c, a) })) + ">" },
        YFn(ps, r, es) => "(" + join(map(ps, fn(a: Ty) -> str { show_ty(c, a) })) + ") -> " + show_ty(c, r) +
            (if len(es) == 0 { "" } else { " ! " + join(map(es, fn(a: Ty) -> str { show_ty(c, a) })) }),
        YRow(es) => join(map(es, fn(a: Ty) -> str { show_ty(c, a) })),
        YVar(id) => "?" + to_str(id),
        YParam(n) => n,
    }
}

fn join(xs: Vec<str>) -> str {
    let mut s = ""
    for i in 0..len(xs) { s = s + (if i > 0 { ", " } else { "" }) + xs[i] }
    s
}

// 二つの型を等しくする。never はどの型とも等しい
partial fn unify(c: C, a0: Ty, b0: Ty) -> bool {
    let a = walk(c, a0)
    let b = walk(c, b0)
    match a {
        YNever => true,
        YVar(i) => { match b { YVar(j) => if i != j { set_var(c, i, b) }, _ => set_var(c, i, b) }; true }
        _ => match b {
            YNever => true,
            YVar(j) => { set_var(c, j, a); true }
            _ => same_shape(c, a, b),
        },
    }
}

partial fn unify_all(c: C, xs: Vec<Ty>, ys: Vec<Ty>) -> bool {
    if len(xs) != len(ys) { return false }
    let mut ok = true
    for i in 0..len(xs) { if i < len(ys) { ok = unify(c, xs[i], ys[i]) && ok } }
    ok
}

partial fn same_shape(c: C, a: Ty, b: Ty) -> bool {
    match a {
        YInt => match b { YInt => true, _ => false },
        YBool => match b { YBool => true, _ => false },
        YStr => match b { YStr => true, _ => false },
        YUnit => match b { YUnit => true, _ => false },
        YType => match b { YType => true, _ => false },
        YVec(x) => match b { YVec(y) => unify(c, x, y), _ => false },
        YRef(x) => match b { YRef(y) => unify(c, x, y), _ => false },
        YData(n, xs) => match b { YData(m, ys) => n == m && unify_all(c, xs, ys), _ => false },
        YFn(ps, r, es) => match b { YFn(qs, s, fs) => unify_all(c, ps, qs) && unify(c, r, s) && unify_effs(c, es, fs), _ => false },
        YParam(n) => match b { YParam(m) => n == m, _ => false },
        _ => false,
    }
}

// 効果の並びを展開する。決まった効果の変数は中身に、決まっていない変数は空にする
partial fn expand(c: C, es: Vec<Ty>) -> Vec<Ty> {
    let mut r: Vec<Ty> = []
    for e in es {
        match walk(c, e) {
            YRow(xs) => { for x in expand(c, xs) { r = push(r, x) } }
            YVar(i) => {}
            _ => { r = push(r, walk(c, e)) }
        }
    }
    r
}

fn eff_name(t: Ty) -> str { match t { YData(n, xs) => n, YParam(n) => n, _ => "" } }

// want の並びが got の効果をすべて含むか。一方が一つの決まっていない変数なら、もう一方の並びにする
partial fn unify_effs(c: C, want: Vec<Ty>, got: Vec<Ty>) -> bool {
    if len(want) == 1 && is_var(walk(c, want[0])) { return unify(c, want[0], YRow(got)) }
    if len(got) == 1 && is_var(walk(c, got[0])) { return unify(c, got[0], YRow(want)) }
    let ws = expand(c, want)
    let mut ok = true
    for g in expand(c, got) { if !covers(c, ws, g) { ok = false } }
    ok
}

fn is_var(t: Ty) -> bool { match t { YVar(i) => true, _ => false } }

partial fn covers(c: C, ws: Vec<Ty>, g: Ty) -> bool {
    let n = eff_name(g)
    if n == "Async" { return true }
    let mut ok = false
    for w in ws { if !ok && eff_name(w) == n { ok = unify(c, w, g) } }
    ok
}

partial fn raise(c: C, es: Vec<Ty>) {
    let mut r = read(c.raised)
    for e in es {
        for d in read(c.declared) { if eff_name(d) != "" && eff_name(d) == eff_name(walk(c, e)) { unify(c, d, e); } }
        r = push(r, e)
    }
    write(c.raised, r)
}

// want の型が必要な所に got の型の式を置く
partial fn expect_ty(c: C, want: Ty, got: Ty, what: str) {
    if !unify(c, want, got) { err(c, what + ": " + show_ty(c, want) + " が必要だが " + show_ty(c, got)); }
}

// ---- 宣言を探す ----

struct CtorOf { d: Data, ctor: Ctor }

fn find_data(c: C, name: str) -> Option<Data> {
    let mut r: Option<Data> = None
    for d in c.prog.datas { if d.name == name { r = Some(d) } }
    r
}

fn find_ctor(c: C, name: str) -> Option<CtorOf> {
    let mut r: Option<CtorOf> = None
    for d in c.prog.datas { for k in d.ctors { if k.name == name { r = Some(CtorOf { d: d, ctor: k }) } } }
    r
}

fn find_fn(c: C, name: str) -> Option<Func> {
    let mut r: Option<Func> = None
    for f in c.prog.fns { if f.name == name { r = Some(f) } }
    r
}

fn is_alias(f: Func) -> bool {
    match f.ret { Some(t) => match t { EVar(n) => n == "Type", _ => false }, None => false }
}

fn param_name(e: Expr) -> str { match e { EParam(n, t) => n, _ => "" } }

fn param_ty(e: Expr) -> Option<Expr> { match e { EParam(n, t) => t, _ => None } }

fn lookup_bind(bs: Vec<Bind>, name: str) -> Option<Ty> {
    let mut r: Option<Ty> = None
    for b in bs { if b.name == name { r = Some(b.ty) } }
    r
}

fn tail_of(e: Expr) -> Option<Expr> { match e { EBlock(st, t) => t, _ => None } }

// ---- 型の式を型にする ----

partial fn ty_of(c: C, env: Vec<Bind>, e: Expr) -> Ty {
    match e {
        EVar(n) => match lookup_bind(env, n) {
            Some(t) => t,
            None => if n == "int" { YInt } else if n == "bool" { YBool } else if n == "str" { YStr }
                else if n == "unit" { YUnit } else if n == "never" { YNever } else if n == "Type" { YType } else if n == "Async" { YData(n, []) }
                else { named_ty(c, env, n, []) }
        },
        ECall(n, args) => named_ty(c, env, n, map(args, fn(a: Expr) -> Ty { ty_of(c, env, a) })),
        ERefine(t, x, cond) => ty_of(c, env, t),
        EPi(ps, r, effs) => {
            let mut env2 = env
            let mut tys: Vec<Ty> = []
            for p in ps {
                let t = match param_ty(p) { Some(x) => ty_of(c, env2, x), None => fresh(c) }
                tys = push(tys, t)
            }
            YFn(tys, ty_of(c, env2, r), map(effs, fn(x: Expr) -> Ty { ty_of(c, env2, x) }))
        }
        _ => err(c, "型でない式"),
    }
}

partial fn named_ty(c: C, env: Vec<Bind>, n: str, args: Vec<Ty>) -> Ty {
    if n == "Vec" && len(args) == 1 { return YVec(args[0]) }
    if n == "Ref" && len(args) == 1 { return YRef(args[0]) }
    match find_data(c, n) {
        Some(d) => if len(d.params) == len(args) { YData(n, args) } else { err(c, "型 " + n + " の引数の数が違う") },
        None => match find_fn(c, n) {
            Some(f) => if is_alias(f) && len(f.params) == len(args) {
                let mut bs: Vec<Bind> = []
                for i in 0..len(f.params) { if i < len(args) { bs = push(bs, Bind { name: param_name(f.params[i]), ty: args[i] }) } }
                match tail_of(f.body) { Some(t) => ty_of(c, bs, t), None => err(c, "型の別名 " + n + " に本体が無い") }
            } else { err(c, n + " は型でない") },
            None => err(c, "不明な型 " + n),
        },
    }
}

// 型引数を新しい型変数にした、データ型の引数の対応
fn data_binds(c: C, d: Data) -> Vec<Bind> {
    map(d.params, fn(p: Expr) -> Bind { Bind { name: param_name(p), ty: fresh(c) } })
}

// 関数の型。ジェネリック引数は新しい型変数にする（明示した型引数があれば targs）
fn find_traitdef(c: C, name: str) -> Option<Func> {
    let mut r: Option<Func> = None
    for f in c.prog.fns { if f.kind == "traitdef" && f.name == name { r = Some(f) } }
    r
}

// <A: T1 + T2> の各トレイトを制約にする
partial fn add_bounds(c: C, t: Expr, ty: Ty) {
    match t {
        EBinary(op, a, b) => { add_bounds(c, a, ty); add_bounds(c, b, ty) },
        EVar(n) => match find_traitdef(c, n) {
            Some(f) => write(c.bounds, push(read(c.bounds), Bound { methods: map(f.params, param_name), ty: ty, call: false })),
            None => {},
        },
        _ => {},
    }
}

fn key_of(c: C, t: Ty) -> str {
    match walk(c, t) { YInt => "int", YStr => "str", YBool => "bool", YVec(x) => "Vec", YData(n, xs) => n, _ => "" }
}

fn has_impl(c: C, ms: Vec<str>, key: str, call: bool) -> bool {
    let mut ok = false
    for f in c.prog.fns {
        for m in ms {
            if (f.kind == "impl" && f.name == m + "#" + key) || (call && f.kind == "default" && f.name == m + "#default") { ok = true }
        }
    }
    ok
}

// ponytail: 型変数のまま残った型と YParam は検査しない。制約は「メソッドのどれかの impl がある」までしか見ない
partial fn check_bounds(c: C) {
    for b in read(c.bounds) {
        let k = key_of(c, b.ty)
        if k != "" && !has_impl(c, b.methods, k, b.call) { err(c, "型 " + show_ty(c, b.ty) + " に " + join_s(b.methods, "・") + " の impl が無い"); }
    }
}

partial fn inst_fn(c: C, f: Func, targs: Vec<Ty>) -> Ty {
    let mut bs: Vec<Bind> = []
    let mut ps: Vec<Ty> = []
    for i in 0..len(f.params) {
        let p = f.params[i]
        if i < f.implicit {
            let bt = if i < len(targs) { targs[i] } else { fresh(c) }
            bs = push(bs, Bind { name: param_name(p), ty: bt })
            match param_ty(p) { Some(t) => add_bounds(c, t, bt), None => {} }
        } else {
            ps = push(ps, match param_ty(p) { Some(t) => ty_of(c, bs, t), None => fresh(c) })
        }
    }
    if f.kind == "trait" { for b in bs { write(c.bounds, push(read(c.bounds), Bound { methods: [f.name], ty: b.ty, call: true })) } }
    YFn(ps, match f.ret { Some(t) => ty_of(c, bs, t), None => YUnit }, map(f.effs, fn(x: Expr) -> Ty { ty_of(c, bs, x) }))
}

partial fn inst_ctor(c: C, k: CtorOf) -> Ty {
    let bs = data_binds(c, k.d)
    let ps = map(k.ctor.fields, fn(p: Expr) -> Ty { match param_ty(p) { Some(t) => ty_of(c, bs, t), None => YNever } })
    YFn(ps, YData(k.d.name, map(bs, fn(b: Bind) -> Ty { b.ty })), [])
}

// ---- 式 ----

fn lookup(env: Vec<Local>, name: str) -> Option<Local> {
    let mut r: Option<Local> = None
    for l in env { if l.name == name { r = Some(l) } }
    r
}

// 引数の型を確かめて戻り値の型を返す
partial fn apply_ty(c: C, env: Vec<Local>, name: str, ft: Ty, args: Vec<Expr>) -> Ty {
    match walk(c, ft) {
        YFn(ps, r, es) => if len(ps) != len(args) {
            err(c, name + " の引数は " + to_str(len(ps)) + " 個だが " + to_str(len(args)) + " 個渡した")
        } else {
            for i in 0..len(args) { if i < len(ps) { expect_ty(c, ps[i], synth(c, env, args[i]), name + " の引数") } }
            raise(c, es)
            r
        }
        YNever => YNever,
        _ => err(c, name + " は関数でない（" + show_ty(c, ft) + "）"),
    }
}

partial fn builtin(c: C, env: Vec<Local>, name: str, args: Vec<Expr>) -> Option<Ty> {
    let a = fresh(c)
    let sig = if name == "print" { Some(YFn([a], YUnit, [])) }
        else if name == "to_str" { Some(YFn([a], YStr, [])) }
        else if name == "push" { Some(YFn([YVec(a), a], YVec(a), [])) }
        else if name == "set_at" { Some(YFn([YVec(a), YInt, a], YVec(a), [])) }
        else if name == "split" { Some(YFn([YStr, YStr], YVec(YStr), [])) }
        else if name == "starts_with" { Some(YFn([YStr, YStr], YBool, [])) }
        else if name == "slice" { Some(YFn([YStr, YInt, YInt], YStr, [])) }
        else if name == "chars" { Some(YFn([YStr], YVec(YStr), [])) }
        else if name == "code" { Some(YFn([YStr], YInt, [])) }
        else if name == "panic" { Some(YFn([YStr], YNever, [])) }
        else if name == "new_ref" { Some(YFn([a], YRef(a), [])) }
        else if name == "read" { Some(YFn([YRef(a)], a, [])) }
        else if name == "write" { Some(YFn([YRef(a), a], YUnit, [])) }
        else if name == "all" { Some(YFn([YVec(YFn([], a, [YData("Async", [])]))], YVec(a), [YData("Async", [])])) }
        else { None }
    match sig {
        Some(t) => Some(apply_ty(c, env, name, t, args)),
        // len は Vec と str の両方
        // resume(x) は腕の操作の戻り値を渡す。resume の後には戻らない
        None => if name == "resume" && len(args) <= 1 {
            let t = if len(args) == 1 { synth(c, env, args[0]) } else { YUnit }
            expect_ty(c, read(c.resume), t, "resume の引数")
            Some(YNever)
        } else if name == "len" && len(args) == 1 {
            match walk(c, synth(c, env, args[0])) {
                YStr => Some(YInt),
                _ => { expect_ty(c, YVec(fresh(c)), synth(c, env, args[0]), "len の引数"); Some(YInt) }
            }
        } else { None },
    }
}

partial fn call(c: C, env: Vec<Local>, name: str, args: Vec<Expr>) -> Ty {
    match lookup(env, name) {
        Some(l) => apply_ty(c, env, name, l.ty, args),
        None => match find_fn(c, name) {
            Some(f) => if f.implicit > 0 && len(args) == len(f.params) {
                // f<型, ...>(引数): 先頭の引数は型
                let mut targs: Vec<Ty> = []
                let mut rest: Vec<Expr> = []
                for i in 0..len(args) { if i < f.implicit { targs = push(targs, ty_of(c, [], args[i])) } else { rest = push(rest, args[i]) } }
                apply_ty(c, env, name, inst_fn(c, f, targs), rest)
            } else { apply_ty(c, env, name, inst_fn(c, f, []), args) }
            None => match find_ctor(c, name) {
                Some(k) => apply_ty(c, env, name, inst_ctor(c, k), args),
                None => match builtin(c, env, name, args) { Some(t) => t, None => err(c, "未定義の関数 " + name) },
            },
        },
    }
}

partial fn synth(c: C, env: Vec<Local>, e: Expr) -> Ty {
    match e {
        EInt(n) => YInt,
        EBool(b) => YBool,
        EStr(s) => YStr,
        EVar(n) => match lookup(env, n) {
            Some(l) => l.ty,
            None => match find_fn(c, n) {
                Some(f) => inst_fn(c, f, []),
                None => match find_ctor(c, n) {
                    Some(k) => match inst_ctor(c, k) { YFn(ps, r, es) => if len(ps) == 0 { r } else { YFn(ps, r, es) }, _ => YNever },
                    None => err(c, "未定義の変数 " + n),
                },
            },
        },
        EUnary(op, x) => if op == "-" { expect_ty(c, YInt, synth(c, env, x), "-"); YInt } else { expect_ty(c, YBool, synth(c, env, x), "!"); YBool },
        EBinary(op, a, b) => binary_ty(c, env, op, a, b),
        ECall(f, args) => call(c, env, f, args),
        EApply(f, args) => apply_ty(c, env, "式の値", synth(c, env, f), args),
        EField(x, f) => field_ty(c, synth(c, env, x), f),
        EMethod(x, f, args) => err(c, "メソッドの呼び出しは未対応"),
        EAssign(n, x) => match lookup(env, n) {
            Some(l) => if !l.mutable { err(c, "変数 " + n + " は mut でない") } else { expect_ty(c, l.ty, synth(c, env, x), "変数 " + n); YUnit },
            None => err(c, "未定義の変数 " + n),
        },
        EIf(cond, t, els) => {
            expect_ty(c, YBool, synth(c, env, cond), "if の条件")
            let tt = synth(c, env, t)
            match els {
                None => YUnit,
                Some(x) => {
                    let et = synth(c, env, x)
                    expect_ty(c, tt, et, "if の分岐")
                    match walk(c, tt) { YNever => et, _ => tt }
                }
            }
        }
        EWhile(cond, body) => { expect_ty(c, YBool, synth(c, env, cond), "while の条件"); synth(c, env, body); YUnit }
        EBlock(stmts, tail) => block_ty(c, env, stmts, tail),
        ELet(n, m, t, init) => err(c, "let はブロックの中にだけ書ける"),
        EReturn(x) => {
            let t = match x { Some(v) => synth(c, env, v), None => YUnit }
            expect_ty(c, read(c.ret), t, "return")
            YNever
        }
        EArray(items) => {
            let a = fresh(c)
            for x in items { expect_ty(c, a, synth(c, env, x), "配列の要素") }
            YVec(a)
        }
        EIndex(v, i) => {
            let a = fresh(c)
            expect_ty(c, YVec(a), synth(c, env, v), "添字の対象")
            expect_ty(c, YInt, synth(c, env, i), "添字")
            a
        }
        EMatch(x, arms) => match_ty(c, env, synth(c, env, x), arms),
        EHandle(x, arms) => handle_ty(c, env, x, arms),
        ELam(ps, ret, effs, body) => {
            let mut env2 = env
            let mut tys: Vec<Ty> = []
            for p in ps {
                let t = match param_ty(p) { Some(x) => ty_of(c, read(c.tparams), x), None => fresh(c) }
                tys = push(tys, t)
                env2 = push(env2, Local { name: param_name(p), ty: t, mutable: false })
            }
            // 無名関数の中の return は無名関数から戻る。本体の効果は無名関数の型の効果にする
            let saved = read(c.ret)
            let saved_raised = read(c.raised)
            write(c.raised, [])
            let r = match ret { Some(x) => ty_of(c, read(c.tparams), x), None => fresh(c) }
            write(c.ret, r)
            expect_ty(c, r, synth(c, env2, body), "無名関数の戻り値")
            write(c.ret, saved)
            let got = expand(c, read(c.raised))
            write(c.raised, saved_raised)
            let es = if len(effs) == 0 { got } else {
                let ws = map(effs, fn(x: Expr) -> Ty { ty_of(c, read(c.tparams), x) })
                for g in got { if !covers(c, ws, g) { err(c, "無名関数が宣言していない効果 " + show_ty(c, g) + " を起こす"); } }
                ws
            }
            YFn(tys, r, es)
        }
        EFor(x, a, b, body) => {
            let xt = match b {
                Some(hi) => { expect_ty(c, YInt, synth(c, env, a), "for の範囲"); expect_ty(c, YInt, synth(c, env, hi), "for の範囲"); YInt }
                None => { let el = fresh(c); expect_ty(c, YVec(el), synth(c, env, a), "for の対象"); el }
            }
            synth(c, push(env, Local { name: x, ty: xt, mutable: false }), body)
            YUnit
        }
        _ => err(c, "型を付けられない式"),
    }
}

partial fn binary_ty(c: C, env: Vec<Local>, op: str, a: Expr, b: Expr) -> Ty {
    let at = synth(c, env, a)
    let bt = synth(c, env, b)
    if op == "&&" || op == "||" {
        expect_ty(c, YBool, at, op); expect_ty(c, YBool, bt, op); YBool
    } else if op == "==" || op == "!=" {
        expect_ty(c, at, bt, op); YBool
    } else if op == "<" || op == "<=" || op == ">" || op == ">=" {
        expect_ty(c, YInt, at, op); expect_ty(c, YInt, bt, op); YBool
    } else if op == "+" && is_str(walk(c, at)) {
        expect_ty(c, YStr, bt, op); YStr
    } else {
        expect_ty(c, YInt, at, op); expect_ty(c, YInt, bt, op); YInt
    }
}

partial fn field_ty(c: C, t: Ty, f: str) -> Ty {
    match walk(c, t) {
        YData(n, args) => match find_ctor(c, n + "{}") {
            Some(k) => {
                let mut bs: Vec<Bind> = []
                for i in 0..len(k.d.params) { if i < len(args) { bs = push(bs, Bind { name: param_name(k.d.params[i]), ty: args[i] }) } }
                let mut r: Option<Ty> = None
                for p in k.ctor.fields { if param_name(p) == f { r = match param_ty(p) { Some(x) => Some(ty_of(c, bs, x)), None => None } } }
                match r { Some(x) => x, None => err(c, "構造体 " + n + " にフィールド " + f + " は無い") }
            }
            None => err(c, n + " は構造体でない"),
        },
        YNever => YNever,
        _ => err(c, "フィールド " + f + " を持たない型 " + show_ty(c, t)),
    }
}

partial fn match_ty(c: C, env: Vec<Local>, t: Ty, arms: Vec<Expr>) -> Ty {
    let r = fresh(c)
    let mut result = YNever
    for arm in arms {
        match arm {
            EArm(ctor, binds, body) => {
                let mut env2 = env
                if ctor != "_" {
                    match find_ctor(c, ctor) {
                        Some(k) => match inst_ctor(c, k) {
                            YFn(ps, dt, es) => {
                                expect_ty(c, dt, t, "match の構築子 " + ctor)
                                if len(ps) != len(binds) { err(c, "構築子 " + ctor + " のフィールドは " + to_str(len(ps)) + " 個"); }
                                for i in 0..len(binds) { if i < len(ps) { env2 = push(env2, Local { name: binds[i], ty: ps[i], mutable: false }) } }
                            }
                            _ => {}
                        }
                        None => { err(c, "不明な構築子 " + ctor); }
                    }
                }
                let bt = synth(c, env2, body)
                expect_ty(c, r, bt, "match の腕")
                match walk(c, bt) { YNever => {}, _ => { result = r } }
            }
            _ => {}
        }
    }
    result
}

// 本体で起きた効果のうち、腕の操作の効果を取り除く。腕の引数の型は、本体で起きた効果の型引数から決まる
partial fn handle_ty(c: C, env: Vec<Local>, x: Expr, arms: Vec<Expr>) -> Ty {
    let saved = read(c.raised)
    write(c.raised, [])
    let bt = synth(c, env, x)
    let inner = expand(c, read(c.raised))
    write(c.raised, saved)
    let mut handled: Vec<str> = []
    for arm in arms {
        match arm {
            EArm(op, binds, body) => match find_fn(c, op) {
                Some(f) => if f.kind != "op" { err(c, op + " は効果の操作でない"); } else {
                    match inst_fn(c, f, []) {
                        YFn(ps, r, es) => {
                            for e in es {
                                handled = push(handled, eff_name(e))
                                for t in inner { if eff_name(t) == eff_name(e) { unify(c, e, t); } }
                            }
                            if len(ps) != len(binds) { err(c, "操作 " + op + " の引数は " + to_str(len(ps)) + " 個"); }
                            let mut env2 = env
                            for i in 0..len(binds) { if i < len(ps) { env2 = push(env2, Local { name: binds[i], ty: ps[i], mutable: false }) } }
                            let saved_r = read(c.resume)
                            write(c.resume, r)
                            let at = synth(c, env2, body)
                            write(c.resume, saved_r)
                            expect_ty(c, bt, at, "handle の腕")
                        }
                        _ => {}
                    }
                }
                None => { err(c, "不明な操作 " + op); }
            },
            _ => {}
        }
    }
    raise(c, filter(inner, fn(t: Ty) -> bool { !contains(handled, eff_name(t)) }))
    bt
}

partial fn block_ty(c: C, env0: Vec<Local>, stmts: Vec<Expr>, tail: Option<Expr>) -> Ty {
    let mut env = env0
    let mut diverges = false
    for s in stmts {
        match s {
            ELet(n, m, t, init) => {
                let it = synth(c, env, init)
                let lt = match t { Some(x) => { let w = ty_of(c, read(c.tparams), x); expect_ty(c, w, it, "変数 " + n); w }, None => it }
                env = push(env, Local { name: n, ty: lt, mutable: m })
            }
            _ => match walk(c, synth(c, env, s)) { YNever => { diverges = true }, _ => {} },
        }
    }
    match tail {
        Some(x) => synth(c, env, x),
        None => if diverges { YNever } else { YUnit },
    }
}

// ---- 関数 ----

partial fn check_fn(c: C, f: Func) {
    write(c.fname, f.name)
    write(c.bounds, [])
    write(c.subst, [])
    let mut bs: Vec<Bind> = []
    let mut env: Vec<Local> = []
    for i in 0..len(f.params) {
        let p = f.params[i]
        if i < f.implicit {
            bs = push(bs, Bind { name: param_name(p), ty: YParam(param_name(p)) })
        } else {
            env = push(env, Local { name: param_name(p), ty: match param_ty(p) { Some(t) => ty_of(c, bs, t), None => YNever }, mutable: false })
        }
    }
    write(c.tparams, bs)
    let r = match f.ret { Some(t) => ty_of(c, bs, t), None => YUnit }
    write(c.ret, r)
    write(c.raised, [])
    let ws = map(f.effs, fn(x: Expr) -> Ty { ty_of(c, bs, x) })
    write(c.declared, ws)
    let bt = synth(c, env, f.body)
    match r { YUnit => {}, _ => expect_ty(c, r, bt, "戻り値") }
    for g in expand(c, read(c.raised)) { if !covers(c, ws, g) { err(c, "効果 " + show_ty(c, g) + " を処理していない（! で宣言するか handle で処理する）"); } }
    check_bounds(c)
}

// 型エラーの並び（無ければ空）
partial fn check(prog: Program) -> Vec<str> {
    let c = C { prog: prog, subst: new_ref([]), errs: new_ref([]), fname: new_ref(""), ret: new_ref(YUnit), tparams: new_ref([]), raised: new_ref([]), resume: new_ref(YUnit), declared: new_ref([]), bounds: new_ref([]) }
    // トレイトのメソッドと JS の関数は本体が無い
    for f in prog.fns { if !is_alias(f) && f.kind != "trait" && f.kind != "traitdef" && f.kind != "op" && !starts_with(f.kind, "js:") { check_fn(c, f) } }
    read(c.errs)
}

test "infers generics and closures" {
    assert_eq(check(parse("enum L<A> { N, C(x: A, xs: L<A>) }\nfn len2<A>(l: L<A>) -> int { match l { N => 0, C(x, xs) => 1 + len2(xs) } }\nfn main() { let f = fn(x) { x + 1 }; print(len2(C(f(1), N))) }")), [])
}

test "reports errors" {
    assert_eq(check(parse("fn f(x: int) -> str { x }")), ["関数 f: 戻り値: str が必要だが int"])
    assert_eq(check(parse("fn f() { let x = 1; x = 2 }")), ["関数 f: 変数 x は mut でない"])
}

fn is_str(t: Ty) -> bool { match t { YStr => true, _ => false } }
