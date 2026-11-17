#[path = "traits.rs"]
mod traits;
#[path = "effects.rs"]
mod effects;
#[path = "proof.rs"]
mod proof;
use effects::{eff_key, flat, infer_effects, op_site};
use traits::is_impl;

// 型検査。型別名と細分化型の条件を評価して比べる
use crate::meta::{Kinds, Q};
use crate::parser::{Arm, Block, Data, Expr, Fn, Name, NodeId, Program, Stmt};
use crate::smt::Smt;
use super::prove::{bin, len, replace_len, un, Env, Eval, Halt, Term};
use std::collections::HashMap;
use std::rc::Rc;

use crate::error::{Ctx, Error, R};
use crate::lexer::Span;

// 型検査中の値。型そのものも値として表す
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum V {
    Int(i64),
    Num(f64),
    Bool(bool),
    Str(Rc<str>),
    Unit,
    Arr(Rc<Vec<V>>),
    // 型を表す印
    Type,
    // data の値 (構築子, フィールド)
    Data(Rc<(Name, Vec<V>)>),
    // data の型 (型名, 引数)
    DataT(Rc<(Name, Vec<V>)>),
    // int・bool・String・syn・unit・never。never は return の型で、どの型とも一致する
    Prim(&'static str),
    // Vec<要素の型>
    VecT(Rc<V>),
    // 細分化型 base[x -> 条件]。条件は x を Var(0) として評価した値
    Refine(Rc<(V, V)>),
    // 関数型 (x: A, ...) -> B
    Pi(Rc<PiV>),
    // 種類の分かっているコード片 syn<種類>。種類の分からないものは Prim("syn")
    Syn(Rc<str>),
    // 値が分からず計算が止まった項
    Neu(Rc<Term>),
}

// 関数型の値。型は引数の値に依存しない
#[derive(Debug, PartialEq)]
pub(crate) struct PiV {
    pub(super) doms: Vec<V>,
    pub(super) cod: V,
    // 呼ぶと起きる効果
    pub(super) effs: Vec<V>,
    // 呼ぶ時に、呼ぶ側のスコープから渡す要求（トレイトの名前）。引数の後ろに、要求の値、実装の順に足す
    pub(super) reqs: Vec<Name>,
}

// js は JavaScript の値をそのまま持ち回る型（extern で使う）。never は戻らない効果の操作（fail など）の戻り値の型
// Effect は効果の並びの型（<E: Effect> の効果の変数に使う）
pub(super) const PRIMS: &[&str] = &["int", "num", "number", "bool", "String", "syn", "unit", "js", "never", "Effect"];

// マクロ本体の中でだけ使える組み込み関数 (名前, 引数, 戻り値)
pub(super) const SYN_FNS: &[(&str, &[&str], &str)] = &[
    ("at", &["syn", "int"], "syn"),
    ("is_null", &["syn"], "bool"),
    ("int_of", &["syn"], "int"),
    ("str_of", &["syn"], "String"),
    ("error", &["String"], "never"),
    // 文字列から識別子のコード片を作る。呼び出し側から見える名前を導入する時に使う
    ("ident", &["String"], "syn"),
];

// どこでも使える組み込み関数
const BUILTINS: &[&str] = &[
    "put",
    "input",
    "refine",
    "num_floor",
    "num_sqrt",
    "len",
    "Vec",
    "Type",
    "to_str",
    "split",
    "starts_with",
    "slice",
    "chars",
    "code",
    "char_from_codepoint",
    "now_ms",
    "panic",
    "resume",
    "all",
    "push",
    "set_at",
    "Ref",
    "new_ref",
    "read",
    "write",
    "Async",
    "Task",
    "spawn",
    "task_then",
    "task_done",
    "task_value",
    "task_wait",
];

// 文字列の組み込み関数 (名前, 引数, 戻り値)。split の戻り値は Vec<String>
const STR_FNS: &[(&str, &[&str], &str)] = &[
    ("num_floor", &["num"], "int"),
    ("num_sqrt", &["num"], "num"),
    ("input", &[], "String"),
    ("split", &["String", "String"], "Vec<String>"),
    ("starts_with", &["String", "String"], "bool"),
    ("slice", &["String", "int", "int"], "String"),
    // 1文字ずつの Vec と、先頭の文字の番号（空なら -1）
    ("chars", &["String"], "Vec<String>"),
    ("code", &["String"], "int"),
    ("char_from_codepoint", &["int"], "String"),
    // 1970 年からのミリ秒
    ("now_ms", &[], "int"),
    // 実行時エラーにする
    ("panic", &["String"], "never"),
];

pub(super) fn prim(name: &str) -> V {
    let name = if name == "number" { "num" } else { name };
    V::Prim(
        PRIMS
            .iter()
            .chain(&["never"])
            .find(|p| **p == name)
            .unwrap(),
    )
}

fn syn_of(kind: &str) -> V {
    if kind.is_empty() {
        prim("syn")
    } else {
        V::Syn(kind.into())
    }
}

// 書き換えられる参照の型 Ref<T>
pub(super) fn ref_t(a: V) -> V {
    V::DataT(Rc::new(("Ref".into(), vec![a])))
}

fn ref_of(t: &V) -> Option<V> {
    match base(t) {
        V::DataT(d) if d.0 == "Ref" && d.1.len() == 1 => Some(d.1[0].clone()),
        _ => None,
    }
}

// まだ走っていない値の型 Async<T>。効果の Async（引数なし）とは引数の有無で区別する。
// rest は .await の時に起きる Async 以外の効果（Async<T> と書いた型では空）
pub(super) fn async_t(t: V, rest: Vec<V>) -> V {
    let mut args = vec![t];
    if !rest.is_empty() {
        args.push(V::Arr(Rc::new(rest)));
    }
    V::DataT(Rc::new(("Async".into(), args)))
}

// Async<T> の値の型なら (T, 待つ時に起きる Async 以外の効果)
pub(super) fn async_of(t: &V) -> Option<(V, Vec<V>)> {
    match base(t) {
        V::DataT(d) if d.0 == "Async" && !d.1.is_empty() => {
            Some((d.1[0].clone(), d.1.get(1).map(|e| flat(std::slice::from_ref(e))).unwrap_or_default()))
        }
        _ => None,
    }
}

// Async<T> の値を捨てる文は誤り。待つ場所を書き忘れた可能性が高い
// ponytail: 警告の仕組みが無いので型エラーにする。文と関数の本体の最後の値だけを見る（else の無い if の分岐などは見ない）
fn no_drop(t: &V) -> R<()> {
    if async_of(t).is_some() {
        return Err(format!("{} の値を捨てている（.await を付けて待つか、spawn で走らせる）", show(t)).into());
    }
    Ok(())
}

pub(super) fn neu(n: Term) -> V {
    V::Neu(Rc::new(n))
}

// 細分化型の条件の中の変数
pub(super) fn bound() -> V {
    neu(Term::Var(0, "_".into()))
}

fn show(v: &V) -> String {
    match v {
        V::Int(n) => n.to_string(),
        V::Num(n) => n.to_string(),
        V::Bool(b) => b.to_string(),
        V::Str(s) => format!("{s:?}"),
        V::Unit => "()".into(),
        V::Arr(v) => format!("[{}]", v.iter().map(show).collect::<Vec<_>>().join(", ")),
        V::Type => "Type".into(),
        // 待つ時に Async 以外の効果も起こす値
        V::DataT(d) if d.0 == "Async" && d.1.len() == 2 => {
            let rest = flat(&d.1[1..]).iter().map(show).collect::<Vec<_>>().join(", ");
            format!("Async<{} ! {rest}>", show(&d.1[0]))
        }
        V::DataT(d) if !d.1.is_empty() => format!(
            "{}<{}>",
            d.0,
            d.1.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        V::Data(d) | V::DataT(d) if d.1.is_empty() => d.0.to_string(),
        V::Data(d) | V::DataT(d) => format!(
            "{}({})",
            d.0,
            d.1.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        V::Prim(p) => p.to_string(),
        V::VecT(a) => format!("Vec<{}>", show(a)),
        V::Refine(r) => format!("{}[_ -> {}]", show(&r.0), show(&r.1)),
        V::Pi(p) => {
            let effs = flat(&p.effs).iter().map(show).collect::<Vec<_>>().join(", ");
            let reqs = p.reqs.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(", ");
            format!(
                "({}) -> {}{}{}",
                p.doms.iter().map(show).collect::<Vec<_>>().join(", "),
                show(&p.cod),
                if effs.is_empty() { String::new() } else { format!(" ! {effs}") },
                if reqs.is_empty() { String::new() } else { format!(" with {reqs}") },
            )
        }
        V::Syn(k) => format!("syn<{k}>"),
        V::Neu(n) => match &**n {
            Term::Var(_, name) => name.to_string(),
            Term::Un(op, x) => format!("{op}{}", show(x)),
            Term::Bin(op, a, b) => format!("({} {op} {})", show(a), show(b)),
            Term::Call(f, args) if f == "[]" => format!("{}[{}]", show(&args[0]), show(&args[1])),
            // 型の別名は大文字で始まる名前とみなして <> で表す
            Term::Call(f, args) if f.starts_with(|c: char| c.is_ascii_uppercase()) => {
                format!(
                    "{f}<{}>",
                    args.iter().map(show).collect::<Vec<_>>().join(", ")
                )
            }
            // ジェネリック引数は値の計算では () になるので、先頭の () は表示しない
            Term::Call(f, args) => format!(
                "{f}({})",
                args.iter()
                    .skip_while(|a| matches!(a, V::Unit))
                    .map(show)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Term::Proj(x, f) => format!("{}.{f}", show(x)),
            Term::Opaque(_) => "?".into(),
            Term::Num(v) => show(v),
        },
    }
}

// 細分化を外した型と、その条件の列
fn split(v: &V) -> (V, Vec<V>) {
    let mut preds = Vec::new();
    let mut v = v.clone();
    while let V::Refine(r) = &v {
        preds.push(r.1.clone());
        v = r.0.clone();
    }
    (v, preds)
}

fn base(v: &V) -> V {
    split(v).0
}

// 効果の名前。効果の変数（<E: Effect>）は変数ごとに別の名前
// 変数 id が値の中に現れるか
fn mentions(v: &V, id: usize) -> bool {
    match v {
        V::Data(d) | V::DataT(d) => d.1.iter().any(|x| mentions(x, id)),
        V::Arr(xs) => xs.iter().any(|x| mentions(x, id)),
        V::VecT(a) => mentions(a, id),
        V::Refine(r) => mentions(&r.0, id) || mentions(&r.1, id),
        V::Pi(p) => p.doms.iter().chain(&p.effs).any(|x| mentions(x, id)) || mentions(&p.cod, id),
        V::Neu(n) => match &**n {
            Term::Var(i, _) => *i == id,
            Term::Un(_, x) => mentions(x, id),
            Term::Bin(_, a, b) => mentions(a, id) || mentions(b, id),
            Term::Proj(x, _) => mentions(x, id),
            Term::Call(_, args) => args.iter().any(|a| mentions(a, id)),
            Term::Opaque(_) => false,
            Term::Num(v) => mentions(v, id),
        },
        _ => false,
    }
}

// 評価が止まる理由。Stuck は値の分からない条件で分岐した時など
// 変数の (型, 値, 可変か)
struct Entry {
    ty: V,
    val: V,
    mutable: bool,
}

// 変数と、そこで成り立つと分かっている事実（bool の値）
#[derive(Default)]
struct Scope {
    vars: HashMap<Name, Entry>,
    facts: Vec<V>,
}

// ponytail: 行番号は式（Expr::At）と宣言・腕（Fn・Data・Arm の span）に付く。特定の場所が無い誤り（main 関数が無い、など）には付かない
struct Ck {
    norm: Eval,
    scopes: Vec<Scope>,
    ret: V,
    // マクロ本体（コンパイル時に実行するコード）の中か
    in_macro: bool,
    smt: Smt,
    // マクロ本体の検査で使う、syntax 規則ごとのフィールドの種類
    rules: HashMap<String, Kinds>,
    // 今起こしてよい効果 (効果の名前, 型)。handle の本体の効果の型は、本体で初めて起こした時に決まる
    avail: Vec<(Name, Option<V>)>,
    // 検査中の handle の腕で、resume に渡す値の型
    resume: Vec<V>,
    // 検査中の handle の腕の入れ子の深さ
    in_arm: usize,
    // JavaScript で待つ呼び出しの式と、async にする無名関数の本体（節点の識別子）
    awaits: std::collections::HashSet<NodeId>,
    // 効果の変数を通して効果を起こす呼び出しと、効果の変数を持つ無名関数の本体。多相な関数の async 版で待つ
    var_awaits: std::collections::HashSet<NodeId>,
    // 効果の変数に Async を入れた、多相な関数の呼び出し。async 版を呼ぶ
    async_inst: std::collections::HashSet<NodeId>,
    // Async<T> の値を作った呼び出し。.await の直下ならそのまま待ち、それ以外は引数を閉じ込めた関数に書き換える
    async_calls: std::collections::HashSet<NodeId>,
    // 検査中の .await の直下の式
    direct_await: Option<NodeId>,
    // 効果の節を省いた関数の効果を推論している（本体の複製を検査して、起きた効果を集める）
    inferring: bool,
    // 効果を推論している無名関数の本体で起きた効果
    collected: Option<Vec<V>>,
    // impl から作った、実装の値を作る関数（名前の順）
    impls: Vec<Fn>,
    // 効果の名前（Async と effect 宣言）
    effect_names: std::collections::HashSet<Name>,
}

// never はどの型とも一致する。細分化された値は元の型として使える
fn same(a: &V, b: &V) -> bool {
    let never = V::Prim("never");
    if a == b || *a == never || *b == never {
        return true;
    }
    // Vec は書き換えられないので、要素の細分化を外して使ってよい
    match (a, &base(b)) {
        (V::VecT(x), V::VecT(y)) => same(x, y),
        // 種類の分からない syn とは互いに使える
        (V::Prim("syn"), V::Syn(_)) | (V::Syn(_), V::Prim("syn")) => true,
        // 下の階層の型は上の階層の型としても使える
        // 起こす効果の少ない関数は、多い関数として使える
        (V::Pi(x), V::Pi(y)) => {
            let xe = flat(&x.effs);
            x.doms == y.doms && x.cod == y.cod && x.reqs == y.reqs && flat(&y.effs).iter().all(|e| xe.contains(e))
        }
        (a, b) => a == b,
    }
}

fn want(e: &V, got: &V, what: &str) -> R<()> {
    if same(e, got) {
        Ok(())
    } else {
        Err(format!("{what}: {}", mismatch(e, got)).into())
    }
}

// 型の不一致の説明。T の位置の Async<T> には .await の書き忘れを伝える
fn mismatch(e: &V, got: &V) -> String {
    let hint = if async_of(got).is_some_and(|(t, _)| same(e, &t)) {
        format!("（.await を付けるか、{} のまま受け取る）", show(got))
    } else {
        String::new()
    };
    format!("{} が必要だが {}{hint}", show(e), show(got))
}

// 関数 f の本体で、効果 eff の操作を呼んでいる最初の位置
// 文字の挿入・削除・置換の最小回数
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let cur = row[j + 1];
            row[j + 1] = (prev + (x != b[j]) as usize).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

// 式とその部分式をすべて f に渡す
pub(crate) fn walk(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    let block = |b: &Block, f: &mut dyn FnMut(&Expr)| {
        for s in &b.stmts {
            match s {
                Stmt::Let { init, .. } => walk(init, f),
                Stmt::Expr(e) => walk(e, f),
            }
        }
        if let Some(t) = &b.tail {
            walk(t, f);
        }
    };
    match e {
        Expr::At(_, x) | Expr::Id(_, x)
        | Expr::Await(x)
        | Expr::Unary(_, x)
        | Expr::Field(x, _)
        | Expr::Assign(_, x)
        | Expr::Return(Some(x)) => walk(x, f),
        Expr::Binary(_, a, b) | Expr::Index(a, b) | Expr::Refine(a, _, b) => {
            walk(a, f);
            walk(b, f);
        }
        Expr::Call(_, xs) | Expr::Array(xs) => xs.iter().for_each(|x| walk(x, f)),
        Expr::Method(x, _, xs) => {
            walk(x, f);
            xs.iter().for_each(|x| walk(x, f));
        }
        Expr::If(c, t, e) => {
            walk(c, f);
            block(t, f);
            if let Some(e) = e {
                walk(e, f);
            }
        }
        Expr::While(c, b) => {
            walk(c, f);
            block(b, f);
        }
        Expr::Block(b) => block(b, f),
        Expr::Match(s, arms) | Expr::Handle(s, arms) => {
            walk(s, f);
            arms.iter().for_each(|a| walk(&a.body, f));
        }
        Expr::Lam(_, _, _, b) => block(b, f),
        Expr::For(_, a, b, body) => {
            walk(a, f);
            if let Some(b) = b {
                walk(b, f);
            }
            block(body, f);
        }
        Expr::Pi(bs, r, ..) => {
            bs.iter().for_each(|(_, t)| walk(t, f));
            walk(r, f);
        }
        Expr::Apply(x, xs) => {
            walk(x, f);
            xs.iter().for_each(|x| walk(x, f));
        }
        _ => {}
    }
}

// resume が handle の腕の末尾（腕の値になる位置）にだけあるか確かめる
fn assigned(e: &Expr, out: &mut Vec<Name>) {
    walk(e, &mut |x| {
        if let Expr::Assign(n, _) = x {
            out.push(n.clone());
        }
    });
}

// ブロックの中で代入される変数名を集める
fn assigned_block(b: &Block, out: &mut Vec<Name>) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => assigned(init, out),
            Stmt::Expr(e) => assigned(e, out),
        }
    }
    if let Some(t) = &b.tail {
        assigned(t, out);
    }
}

// quote の中の $x.y の先頭の名前を集める
fn q_names(q: &[Q], out: &mut Vec<Name>) {
    for x in q {
        match x {
            Q::Var { path, .. } => out.push(path[0].clone().into()),
            Q::Rep(inner, _) => q_names(inner, out),
            _ => {}
        }
    }
}

// 関数の本体から呼ぶ関数名を集める
pub(crate) fn callees(f: &Fn) -> Vec<Name> {
    let mut out = Vec::new();
    walk(&Expr::Block(f.body.clone()), &mut |x| {
        if let Expr::Call(n, _) | Expr::Var(n) = x {
            out.push(n.clone());
        }
    });
    out
}

// 型検査の結果のうち、出力に使うもの
pub struct Info {
    // JavaScript で待つ呼び出しの式と、async にする無名関数の本体（節点の識別子）
    pub awaits: std::collections::HashSet<NodeId>,
    pub var_awaits: std::collections::HashSet<NodeId>,
    pub async_inst: std::collections::HashSet<NodeId>,
}

// 型検査し、メソッド呼び出しと制約の実装をその場で書き込む
pub fn check(prog: &mut Program) -> R<Info> {
    check_prog(prog, true).map_err(|error| error.with_sources(prog.modules.iter().map(|module| &module.source)).with_line())
}

// main を求めない検査（ライブラリ出力）
pub fn check_library(prog: &mut Program) -> R<Info> {
    check_prog(prog, false).map_err(|error| error.with_sources(prog.modules.iter().map(|module| &module.source)).with_line())
}

// 位置が分かっている宣言・腕の誤りには、その位置を付ける（内側で付いた位置は残す）
fn at_opt(e: Error, s: Option<Span>) -> Error {
    match s {
        Some(s) => e.at(s),
        None => e,
    }
}

fn check_prog(prog: &mut Program, require_main: bool) -> R<Info> {
    let mut ck = Ck {
        norm: Eval::new(),
        scopes: Vec::new(),
        ret: V::Unit,
        in_macro: false,
        smt: Smt::new(),
        rules: HashMap::new(),
        avail: Vec::new(),
        resume: Vec::new(),
        in_arm: 0,
        awaits: Default::default(),
        var_awaits: Default::default(),
        async_inst: Default::default(),
        async_calls: Default::default(),
        direct_await: None,
        inferring: false,
        collected: None,
        impls: {
            let mut v: Vec<Fn> = prog.fns.iter().filter(|f| is_impl(&f.name)).cloned().collect();
            v.sort_by(|a, b| a.name.cmp(&b.name));
            v
        },
        effect_names: std::iter::once(Name::from("Async"))
            .chain(prog.fns.iter().filter_map(|f| f.op.clone()))
            .chain(
                prog.datas
                    .iter()
                    .filter(|d| d.ctors.is_empty())
                    .map(|d| d.name.clone()),
            )
            .collect(),
    };
    // 関数名・型名・構築子名は同じ名前空間にある
    let mut names: Vec<&str> = Vec::new();
    for (n, sp) in prog
        .datas
        .iter()
        .flat_map(|d| {
            std::iter::once((&d.name, d.span)).chain(d.ctors.iter().map(|c| (&c.name, d.span)))
        })
        .chain(prog.fns.iter().map(|f| (&f.name, f.span)))
    {
        let reserved = BUILTINS.contains(&n.as_str())
            || PRIMS.contains(&n.as_str())
            || SYN_FNS.iter().any(|s| s.0 == n);
        if reserved || names.contains(&n.as_str()) {
            return Err(at_opt(format!("名前 {n} が重複している").into(), sp));
        }
        names.push(n);
    }
    for d in &prog.datas {
        ck.norm.datas.insert(d.name.clone(), d.clone());
        for c in &d.ctors {
            ck.norm.ctors.insert(c.name.clone(), (d.clone(), c.clone()));
        }
    }
    for f in &prog.fns {
        ck.norm.fns.insert(f.name.clone(), f.clone());
    }
    for d in &prog.datas { ck.data_level(d).map_err(|e| at_opt(e, d.span))?; }
    if require_main {
        match ck.norm.fns.get(&Name::from("main")) {
            Some(f) if f.params.is_empty() => {}
            _ => return Err("引数なしの main 関数が必要".into()),
        }
    }
    infer_effects(&mut ck, prog);
    for f in &mut prog.fns {
        check_fn(&mut ck, f).map_err(|e| at_opt(e, f.span))?;
    }
    Ok(Info {
        awaits: ck.awaits,
        var_awaits: ck.var_awaits,
        async_inst: ck.async_inst,
    })
}

// 関数 1 つを検査する
fn check_fn(ck: &mut Ck, f: &mut Fn) -> R<()> {
    let r = |e: &str| format!("関数 {}: {e}", f.name);
    ck.scopes = vec![Scope::default()];
    for (n, t) in &f.params {
        let ty = ck.eval_type(t).ctx(|e| r(&format!("引数 {n}: {e}")))?;
        let id = ck.norm.fresh();
        ck.bind(n, ty, neu(Term::Var(id, n.clone())), false).ctx(r)?;
    }
    ck.avail = Vec::new();
    // 効果を推論している関数は、起きた効果を集める。main は Async だけを起こせるので、Async を起こしたかだけを見る
    if ck.inferring && f.name == "main" {
        ck.avail.push((Name::from("Async"), None));
    } else if ck.inferring {
        ck.avail.push((Name::from("*"), None));
        ck.collected = Some(Vec::new());
    }
    let declared = if ck.inferring { &[][..] } else { &f.effects[..] };
    for e in declared {
        let mut env = ck.val_env();
        for v in ck.effect_inst(e, &mut env).ctx(r)? {
            let k = eff_key(&v);
            if f.name == "main" && k != "Async" {
                return Err(r(
                    "main 関数が起こせる効果は Async だけ（ほかの効果は handle で処理する）",
                )
                .into());
            }
            ck.avail.push((k, Some(v)));
        }
    }
    ck.ret = match &f.ret {
        Some(t) => ck.eval_type(t).ctx(|e| r(&format!("戻り値: {e}")))?,
        None => prim("unit"),
    };
    // never はどの型とも一致するので、本体が戻らないこと（never の式で終わり、return を含まない）を確かめる
    let never_fn = f.op.is_none() && ck.ret == prim("never");
    if never_fn {
        let mut ret_found = false;
        let tail = f.body.tail.iter().map(|t| &**t);
        for e in f
            .body
            .stmts
            .iter()
            .map(|s| match s {
                Stmt::Let { init, .. } => init,
                Stmt::Expr(e) => e,
            })
            .chain(tail)
        {
            walk(e, &mut |x| ret_found |= matches!(x, Expr::Return(_)));
        }
        if ret_found {
            return Err(r("戻り値が never の関数では return を使えない").into());
        }
    }
    if !f.alias && matches!(base(&ck.ret), V::Type) {
        return Err(r("関数は型を返せない。型を計算するなら type で定義する").into());
    }
    // extern の関数と効果の操作は本体が無いので、型の宣言だけを検査する
    if f.js.is_some() || f.op.is_some() {
        return Ok(());
    }
    let ret = ck.ret.clone();
    // 戻り値が unit の関数は最後の式の値を捨てる
    if ret == prim("unit") {
        no_drop(&ck.block(&mut f.body).ctx(r)?).ctx(r)?;
    } else if never_fn {
        if ck.block(&mut f.body).ctx(r)? != prim("never") {
            return Err(
                r("戻り値が never の関数は、panic などの戻らない式で終わる必要がある").into(),
            );
        }
    } else {
        ck.check_block(&mut f.body, &ret)
            .ctx(|e| r(&format!("戻り値: {e}")))?;
    }
    Ok(())
}

// マクロ本体を、束縛名を syn 型の変数として検査する。値は syn 型でなければならない
pub fn check_macro(
    b: &mut Block,
    params: &Kinds,
    rules: &HashMap<String, Kinds>,
    prog: &Program,
) -> R<()> {
    check_macro_body(b, params, rules, prog).map_err(Error::with_line)
}

fn check_macro_body(
    b: &mut Block,
    params: &Kinds,
    rules: &HashMap<String, Kinds>,
    prog: &Program,
) -> R<()> {
    let mut ck = Ck {
        norm: Eval::new(),
        scopes: vec![Scope::default()],
        ret: prim("syn"),
        in_macro: true,
        smt: Smt::new(),
        rules: rules.clone(),
        avail: Vec::new(),
        resume: Vec::new(),
        in_arm: 0,
        awaits: Default::default(),
        var_awaits: Default::default(),
        async_inst: Default::default(),
        async_calls: Default::default(),
        direct_await: None,
        inferring: false,
        collected: None,
        impls: Vec::new(),
        effect_names: Default::default(),
    };
    for f in &prog.fns {
        ck.norm.fns.insert(f.name.clone(), f.clone());
    }
    for d in &prog.datas {
        ck.norm.datas.insert(d.name.clone(), d.clone());
        for c in &d.ctors {
            ck.norm.ctors.insert(c.name.clone(), (d.clone(), c.clone()));
        }
    }
    let mut helpers = prog.clone();
    let mut pending = Vec::new();
    walk(&Expr::Block(b.clone()), &mut |e| {
        if let Expr::Call(n, _) | Expr::Var(n) = e {
            pending.push(n.clone());
        }
    });
    let mut checked = std::collections::HashSet::new();
    while let Some(n) = pending.pop() {
        if !checked.insert(n.clone()) {
            continue;
        }
        if let Some(f) = helpers.fns.iter_mut().find(|f| f.name == n) {
            pending.extend(callees(f));
            check_fn(&mut ck, f)?;
        }
    }
    ck.scopes = vec![Scope::default()];
    ck.ret = prim("syn");
    for (p, k) in params {
        let v = ck.norm.opaque();
        ck.bind(&Name::from(p.as_str()), syn_of(k), v, false)?;
    }
    let t = ck.block(b)?;
    want(&prim("syn"), &t, "マクロ本体の値")
}

impl Ck {
    fn block(&mut self, b: &mut Block) -> R<V> {
        self.push();
        let r = self.block_inner(b, None);
        self.pop();
        r
    }

    fn check_block(&mut self, b: &mut Block, expected: &V) -> R<()> {
        self.push();
        let r = self.block_inner(b, Some(expected));
        self.pop();
        r.map(|_| ())
    }

    // expected があれば最後の式をその型で検査する
    fn block_inner(&mut self, b: &mut Block, expected: Option<&V>) -> R<V> {
        let mut diverges = false;
        for s in &mut b.stmts {
            match s {
                Stmt::Let {
                    name,
                    mutable,
                    ty,
                    init,
                } => {
                    let t = match ty {
                        Some(ty) => {
                            let t = self.eval_type(ty)?;
                            self.check(init, &t).ctx(|e| format!("変数 {name}: {e}"))?;
                            t
                        }
                        None => self.expr(init)?,
                    };
                    // 可変な変数は、値の代わりに変数そのものを表す項を持つ
                    let val = if *mutable {
                        let id = self.norm.fresh();
                        neu(Term::Var(id, name.clone()))
                    } else {
                        self.eval_value(init)?
                    };
                    self.bind(name, t.clone(), val.clone(), *mutable)?;
                    // 可変な Vec は、初期値の長さを事実として持つ
                    if *mutable && matches!(base(&t), V::VecT(_)) {
                        let l = len(self.eval_value(init)?);
                        self.add_fact(bin("==", len(val), l));
                    }
                }
                Stmt::Expr(e) => {
                    let t = self.expr(e)?;
                    no_drop(&t)?;
                    diverges |= t == prim("never");
                }
            }
        }
        let t = match (b.tail.as_mut(), expected) {
            (Some(e), Some(t)) => {
                self.check(e, t)?;
                t.clone()
            }
            (Some(e), None) => self.expr(e)?,
            (None, _) if diverges => prim("never"),
            (None, _) => prim("unit"),
        };
        if b.tail.is_none()
            && let Some(e) = expected
        {
            want(e, &t, "ブロックの値")?;
        }
        Ok(t)
    }

    // 分岐の中に入る。条件 c が b であるという事実を加える
    // 期待する型に対して検査する。if は条件の値で期待する型を絞り込む
    fn check(&mut self, e: &mut Expr, expected: &V) -> R<()> {
        if let Expr::At(sp, inner) = e {
            return self.check(inner, expected).map_err(|err| err.at(*sp));
        }
        if let Expr::Id(_, inner) = e {
            return self.check(inner, expected);
        }
        match e {
            Expr::If(c, then, Some(els)) => {
                want(&prim("bool"), &self.expr(c)?, "if の条件")?;
                let cv = self.eval_value(c)?;
                // 条件が値の分からない変数なら、各分岐ではその値が分かっているものとして扱う
                let var = match (c.peel(), &cv) {
                    (Expr::Var(n), V::Neu(nu)) => match &**nu {
                        Term::Var(id, _) => {
                            Some((n.clone(), *id, self.lookup(n).unwrap().ty.clone()))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                for b in [true, false] {
                    let mut exp = expected.clone();
                    self.enter_branch(&cv, b);
                    if let Some((n, id, ty)) = &var {
                        exp = self.norm.subst(expected, *id, &V::Bool(b))?;
                        self.bind(n, ty.clone(), V::Bool(b), false)?;
                    }
                    let r = if b {
                        self.check_block(then, &exp)
                    } else {
                        self.check(els, &exp)
                    };
                    self.pop();
                    r?;
                }
                Ok(())
            }
            Expr::Block(b) => self.check_block(b, expected),
            Expr::Match(s, arms) => self.check_match(s, arms, Some(expected)).map(|_| ()),
            Expr::Call(n, args)
                if self.lookup(n).is_none()
                    && self
                        .norm
                        .fns
                        .get(n)
                        .is_some_and(|f| f.implicit > 0) =>
            {
                let mut call = std::mem::replace(e, Expr::Int(0));
                let (id, expression) = call.identified();
                let Expr::Call(name, args) = expression else { unreachable!() };
                let t = self.call_fn(name, args, Some(expected), id)?;
                self.delay(id, expression);
                *e = call;
                self.subsume(e, &t, expected)
            }
            // 期待する型から、new_ref([]) の要素の型が分かる
            Expr::Call(n, args)
                if n == "new_ref"
                    && self.lookup(n).is_none()
                    && args.len() == 1
                    && ref_of(expected).is_some() =>
            {
                self.check(&mut args[0], &ref_of(expected).unwrap())
            }
            Expr::Lam(params, ret, effs, body) if matches!(base(expected), V::Pi(_)) => {
                let V::Pi(p) = base(expected) else {
                    unreachable!()
                };
                // 書いた効果は、期待する関数型の効果に含まれていなければならない
                for e in effs {
                    let mut env = self.val_env();
                    for v in self.effect_inst(e, &mut env)? {
                        if !flat(&p.effs).contains(&v) {
                            return Err(format!(
                                "{} が必要だが、効果 {} を起こす無名関数",
                                show(expected),
                                show(&v)
                            )
                            .into());
                        }
                    }
                }
                if p.reqs.is_empty() {
                    self.lam_check(params, ret, body, &p)
                } else {
                    self.lam_with_reqs(params, ret, body, &p)
                }
            }
            Expr::Call(n, args) if self.norm.ctors.contains_key(n) => {
                let t = self.ctor_call(n, args, Some(expected))?;
                self.subsume(e, &t, expected)
            }
            Expr::Var(n)
                if self.lookup(n).is_none() && self.norm.ctors.contains_key(n) =>
            {
                let t = self.ctor_call(n, &mut [], Some(expected))?;
                self.subsume(e, &t, expected)
            }
            Expr::Array(items) if matches!(base(expected), V::VecT(_)) => {
                let eb = base(expected);
                let V::VecT(a) = &eb else { unreachable!() };
                for it in items {
                    self.check(it, a)?;
                }
                self.subsume(e, &eb, expected)
            }
            e => {
                let t = self.expr(e)?;
                self.subsume(e, &t, expected)
            }
        }
    }

    fn expr(&mut self, e: &mut Expr) -> R<V> {
        if let Expr::At(sp, inner) = e {
            return self.expr(inner).map_err(|err| err.at(*sp));
        }
        if let Expr::Id(id, inner) = e {
            let id = *id;
            let t = self.expr_inner(inner, id)?;
            self.delay(id, inner);
            return Ok(t);
        }
        if matches!(e, Expr::Call(n, _) if n == "refine") { return self.refine_value(e); }
        // 長い a + b + ... は左に深く再帰するので、大きな expr_inner のフレームを通らずに検査する
        if matches!(e, Expr::Binary(..)) {
            return self.binary(e);
        }
        let (id, expression) = e.identified();
        let t = self.expr_inner(expression, id)?;
        self.delay(id, expression);
        Ok(t)
    }

    #[inline(never)]
    fn binary(&mut self, e: &mut Expr) -> R<V> {
        let Expr::Binary(op, mut l, mut r) = std::mem::replace(e, Expr::Int(0)) else {
            unreachable!()
        };
        let int = prim("int");
        let boolean = prim("bool");
        let lt = self.expr(&mut l)?;
        // a && b の b では a が真、a || b の b では a が偽と分かっている
        let rt = if matches!(op, "&&" | "||") {
            let lv = self.eval_value(&l)?;
            self.enter_branch(&lv, op == "&&");
            let rt = self.expr(&mut r);
            self.pop();
            rt?
        } else {
            self.expr(&mut r)?
        };
        let ty = match op {
            "+" if base(&lt) == prim("String") => {
                want(&prim("String"), &rt, &op)?;
                prim("String")
            }
            "+" | "-" | "*" | "**" | "/" | "%" | "<" | "<=" | ">" | ">=" => {
                let numeric = if base(&lt) == prim("num") || base(&rt) == prim("num") { prim("num") } else { int.clone() };
                if numeric == prim("num") {
                    self.check(&mut l, &numeric)?;
                    self.check(&mut r, &numeric)?;
                } else {
                    want(&int, &lt, &op)?;
                    want(&int, &rt, &op)?;
                }
                if matches!(op, "+" | "-" | "*" | "**" | "/" | "%") {
                    numeric
                } else {
                    boolean
                }
            }
            "&&" | "||" => {
                want(&boolean, &lt, &op)?;
                want(&boolean, &rt, &op)?;
                boolean
            }
            _ => {
                // == と !=
                let lt = base(&lt);
                if (lt == prim("num") && base(&rt) == int) || (lt == int && base(&rt) == prim("num")) {
                    self.check(&mut l, &prim("num"))?;
                    self.check(&mut r, &prim("num"))?;
                    *e = Expr::Binary(op, l, r);
                    return Ok(boolean);
                }
                if lt == prim("num") {
                    self.check(&mut l, &lt)?;
                    self.check(&mut r, &lt)?;
                }
                want(&lt, &base(&rt), &op)?;
                if lt == prim("syn") {
                    return Err("syn は比較できない".into());
                }
                if let Some(inst) = self.trait_inst("eq", &lt)? {
                    let call = Expr::Call("std.eq".into(), vec![*l, *r, inst]);
                    *e = if op == "!=" {
                        Expr::Unary("!".into(), Box::new(call))
                    } else {
                        call
                    };
                    return Ok(boolean);
                }
                boolean
            }
        };
        *e = Expr::Binary(op, l, r);
        Ok(ty)
    }

    // x.f(a) は、x の型 T に属する関数 T::f を T::f(x, a) と呼ぶ
    fn method(&mut self, e: &mut Expr, addr: NodeId) -> R<V> {
        let Expr::Method(mut x, f, xs) = std::mem::replace(e, Expr::Int(0)) else {
            unreachable!()
        };
        let t = self.expr(&mut x)?;
        // 呼び出しの結果に対するメソッドは、結果を一時変数に置いてから呼ぶ。
        // 検査済みの呼び出しをもう一度検査すると、制約の実装の引数を二重に足すため
        if matches!(x.peel(), Expr::Call(..) | Expr::Apply(..)) {
            let tmp = Name::temporary("recv", self.norm.fresh());
            let v = self.norm.opaque();
            self.bind(&tmp, t, v, false)?;
            *e = Expr::Method(Box::new(Expr::Var(tmp.clone())), f, xs);
            let (id, expression) = e.identified();
            let r = self.method(expression, id)?;
            self.delay(id, expression);
            let inner = std::mem::replace(e, Expr::Int(0));
            *e = Expr::Block(Block { id: Default::default(),
                stmts: vec![Stmt::Let { name: tmp, mutable: false, ty: None, init: *x }],
                tail: Some(Box::new(inner)),
            });
            return Ok(r);
        }
        // t.wait() はブロックして待つ。ネイティブでだけ使えるので、JavaScript と WASM の出力が位置を付けて拒む
        if let V::DataT(d) = base(&t)
            && d.0 == "Task"
            && f == "wait"
            && xs.is_empty()
        {
            *e = Expr::Call("task_wait".into(), vec![*x]);
            return Ok(d.1[0].clone());
        }
        let ty = match base(&t) {
            V::DataT(d) => Some(d.0.clone()),
            V::Prim(p) => Some(Name::from(p)),
            V::VecT(_) => Some(Name::from("Vec")),
            _ => None,
        };
        let mut name = ty.as_ref().map(|ty| Name::associated(ty.clone(), f.clone())).unwrap_or_else(|| Name::from(""));
        // 組み込みの型（String など）のメソッドは、どのモジュールで宣言してもよい
        // ponytail: 同じ名前を複数のモジュールが宣言すれば最初の1つ。重なりの検査はトレイトに寄せる時に足す
        if let Some(ty) = &ty
            && !self.norm.fns.contains_key(&name)
            && !matches!(ty.kind, crate::parser::NameKind::Qualified(..))
        {
            if let Some(k) = self.norm.fns.keys().filter(|candidate| candidate.local() == &name).min() {
                name = k.clone();
            }
        }
        // T::f が無ければ、f をメソッドに持つトレイトのうち、x の型に実装があるもの
        if !self.norm.fns.contains_key(&name) {
            let mut traits: Vec<(Name, Name)> = self
                .norm
                .datas
                .values()
                .filter(|d| {
                    matches!(d.ctors.as_slice(), [c] if c.name.structure_owner() == Some(d.name.clone())
                        && c.fields.iter().any(|(n, _)| *n == f))
                })
                .map(|d| (d.name.clone(), crate::parser::qual(&d.module, &f)))
                .collect();
            traits.sort();
            // Self 以外の型引数（trait Fail<E> の E）は、実装に合わせて決める
            let has_impl = traits.iter().find(|(tr, _)| {
                let metas: Vec<usize> = (1..self.norm.datas[tr].params.len()).map(|_| self.norm.fresh()).collect();
                let mut args = vec![base(&t)];
                args.extend(metas.iter().map(|id| neu(Term::Var(*id, "?".into()))));
                let goal = V::DataT(Rc::new((tr.clone(), args)));
                self.resolve_in(&goal, 0, &metas).is_ok()
            });
            match (has_impl, traits.first()) {
                (Some((_, m)), _) => name = m.clone(),
                // 実装が無ければ組み込みの文字列化（std.Display の実装があればそれを使う）
                _ if f == "to_string" && xs.is_empty() => {
                    *e = Expr::Call("to_str".into(), vec![*x]);
                    return self.to_str(e);
                }
                // 実装が無いことは、呼び出しの検査で報告する
                (None, Some((_, m))) => name = m.clone(),
                (None, None) => {
                    let Some(ty) = ty else {
                        return Err(format!("{} の値にはメソッドを呼べない", show(&t)).into());
                    };
                    return Err(format!(
                        "{} にメソッド {f} が無い（fn {}::{f} を宣言する）",
                        show(&t),
                        ty.rsplit('.').next().unwrap_or(&ty)
                    )
                    .into());
                }
            }
        }
        let mut args = Vec::with_capacity(xs.len() + 1);
        args.push(*x);
        args.extend(xs);
        let r = self.call_fn(&name, &mut args, None, addr);
        *e = Expr::Call(name, args);
        r
    }

    // to_str(x) は、std.Display の実装があれば std.to_string(x, 実装) にする
    fn to_str(&mut self, e: &mut Expr) -> R<V> {
        let Expr::Call(_, mut args) = std::mem::replace(e, Expr::Int(0)) else {
            unreachable!()
        };
        let [a] = args.as_mut_slice() else {
            return Err("to_str の引数は1個".into());
        };
        let t = self.expr(a)?;
        if async_of(&t).is_some() {
            return Err(format!("{} の値は文字列にできない（.await を付けて待つ）", show(&t)).into());
        }
        match self.trait_inst("show", &t)? {
            Some(inst) => {
                let a = args.remove(0);
                let shown = Expr::Call("std.to_string".into(), vec![a, inst]);
                *e = shown;
            }
            // 構文片（syn）は、ソースの綴りを文字列にする（str_of と同じ）
            None if self.in_macro && same(&prim("syn"), &t) => *e = Expr::Call("str_of".into(), args),
            None => *e = Expr::Call("to_str".into(), args),
        }
        Ok(prim("String"))
    }

    #[inline(never)]
    fn expr_inner(&mut self, e: &mut Expr, addr: NodeId) -> R<V> {
        match e {
            Expr::At(span, inner) => return self.expr(inner).map_err(|error| error.at(*span)),
            Expr::Id(id, inner) => return self.expr_inner(inner, *id),
            Expr::Binary(..) => return self.binary(e),
            _ => {}
        }
        if matches!(e, Expr::Method(..)) {
            return self.method(e, addr);
        }
        if matches!(e, Expr::Call(n, _) if n == "to_str") {
            return self.to_str(e);
        }

        let int = prim("int");
        let boolean = prim("bool");
        Ok(match &mut *e {
            Expr::At(..) | Expr::Id(..) => unreachable!("先頭で外している"),
            Expr::Method(..) => unreachable!("expr_inner の先頭で分けている"),
            Expr::Int(_) => int,
            Expr::Num(n) => {
                if !n.is_finite() { return Err("num の値は有限でなければならない".into()); }
                prim("num")
            }
            Expr::Bool(_) => boolean,
            Expr::Str(_) => prim("String"),
            Expr::Var(n) => match self.lookup(n) {
                Some(en) => en.ty.clone(),
                None if n == "Type" => V::Type,
                None if PRIMS.contains(&n.as_str()) => V::Type,
                None if self
                    .norm
                    .fns
                    .get(n)
                    .is_some_and(|f| f.alias && f.params.is_empty()) =>
                {
                    let mut call = Expr::Call(n.clone(), vec![]);
                    return self.expr(&mut call);
                }
                None if self.norm.ctors.contains_key(n) => {
                    return self.ctor_call(n, &mut [], None);
                }
                None if self.norm.datas.contains_key(n) => return self.data_app(n, &mut []),
                None if self.norm.fns.contains_key(n) => {
                    let f = self.norm.fns[n].clone();
                    return self.fn_type(&f);
                }
                None => {
                    let e: Error = format!("未定義の変数 {n}").into();
                    return Err(self.suggest(e, n));
                }
            },
            Expr::Field(x, f) => {
                let t = self.expr(x)?;
                if let V::DataT(d) = base(&t) {
                    return self.struct_field(&d, x, f);
                }
                want(&prim("syn"), &t, "フィールド参照")?;
                self.syn_field(&base(&t), f)?
            }
            Expr::Macro(n, _) => return Err(format!("マクロ {n} に展開コードが無い").into()),
            Expr::Quote(q) => {
                if !self.in_macro {
                    return Err("quote はマクロ本体の中でだけ使える".into());
                }
                self.check_q(q)?;
                prim("syn")
            }
            Expr::Array(items) => {
                let Some(first) = items.first_mut() else {
                    return Err("空の配列には型を書く（例: let v: Vec<int> = []）".into());
                };
                let a = base(&self.expr(first)?);
                for it in &mut items[1..] {
                    self.check(it, &a)?;
                }
                V::VecT(Rc::new(a))
            }
            Expr::Index(v, i) => {
                let V::VecT(a) = base(&self.expr(&mut **v)?) else {
                    return Err("添字は Vec にだけ使える".into());
                };
                self.check(i, &int)?;
                self.in_range(v, i)?;
                (*a).clone()
            }
            Expr::Refine(b, x, p) => {
                self.type_level(b)?;
                let bt = self.eval_value(b)?;
                self.push();
                self.bind(x, bt, bound(), false)?;
                let r = self
                    .expr(p)
                    .and_then(|t| want(&boolean, &t, "細分化型の条件"));
                self.pop();
                r?;
                V::Type
            }
            Expr::Unary(op, x) => {
                let xt = self.expr(x)?;
                let t = if *op == "-" && base(&xt) == prim("num") { prim("num") } else if *op == "-" { int } else { boolean };
                want(&t, &xt, op)?;
                if *op == "-" && t == prim("num") { self.check(x, &t)?; }
                t
            }
            Expr::Call(name, args) if name == "#num" => {
                let t = base(&self.expr(&mut args[0])?);
                if t != int && t != prim("num") { return Err("num への変換には int か num が必要".into()); }
                prim("num")
            }
            Expr::Binary(..) => unreachable!("expr で分けている"),
            Expr::Handle(body, arms) => return self.handle(body, arms),
            Expr::Await(_) => return self.await_(e, addr),
            Expr::Call(name, args) if (name == "Async" || name == "Task") && self.lookup(name).is_none() => {
                let [a] = args.as_mut_slice() else {
                    return Err(format!("{name} の引数は1個").into());
                };
                self.type_level(a).ctx(|e| format!("{name} の引数: {e}"))?;
                V::Type
            }
            // spawn(x) は Async<T> の値を走らせ始め、効果を持たない Task<T> を返す。spawn 自体は効果を起こさない
            Expr::Call(name, args) if name == "spawn" && self.lookup(name).is_none() => {
                let [a] = args.as_mut_slice() else {
                    return Err("spawn の引数は1個".into());
                };
                let at = self.expr(a)?;
                let Some((t, rest)) = async_of(&at) else {
                    return Err(format!("spawn には Async<T> の値を渡す（実際は {}）", show(&at)).into());
                };
                if !rest.is_empty() {
                    return Err(format!(
                        "spawn に渡す値は Async のほかの効果を起こせない（{} を起こす）。失敗は本体の中で handle して Result にしてから渡す",
                        flat(&rest).iter().map(show).collect::<Vec<_>>().join(", ")
                    )
                    .into());
                }
                V::DataT(Rc::new(("Task".into(), vec![t])))
            }
            // std の Task のメソッドの実装に使う組み込み
            Expr::Call(name, args)
                if matches!(name.as_str(), "task_then" | "task_done" | "task_value" | "task_wait")
                    && self.lookup(name).is_none() =>
            {
                let n = if name == "task_then" { 2 } else { 1 };
                if args.len() != n {
                    return Err(format!("{name} の引数は{n}個").into());
                }
                let t = self.expr(&mut args[0])?;
                let V::DataT(d) = base(&t).clone() else {
                    return Err(format!("{name} は Task に使う（実際は {}）", show(&t)).into());
                };
                if d.0 != "Task" {
                    return Err(format!("{name} は Task に使う（実際は {}）", show(&t)).into());
                }
                match name.as_str() {
                    // 終わった時に呼ぶ関数は効果を持たない
                    "task_then" => {
                        let f = V::Pi(Rc::new(PiV { doms: vec![d.1[0].clone()], cod: prim("unit"), effs: vec![], reqs: vec![] }));
                        self.check(&mut args[1], &f)?;
                        prim("unit")
                    }
                    "task_done" => boolean,
                    _ => d.1[0].clone(),
                }
            }
            // push(v, x) は v の後ろに x を足した新しい Vec
            Expr::Call(name, args) if name == "push" && self.lookup(name).is_none() => {
                let [v, x] = args.as_mut_slice() else {
                    return Err("push の引数は2個".into());
                };
                let t = base(&self.expr(v)?);
                let V::VecT(a) = &t else {
                    return Err(format!("push は Vec に使う（実際は {}）", show(&t)).into());
                };
                self.check(x, a)?;
                t
            }
            // set_at(v, i, x) は v の i 番目を x にした新しい Vec。i は範囲内であることを証明する
            Expr::Call(name, args) if name == "set_at" && self.lookup(name).is_none() => {
                let [v, i, x] = args.as_mut_slice() else {
                    return Err("set_at の引数は3個".into());
                };
                let t = base(&self.expr(v)?);
                let V::VecT(a) = &t else {
                    return Err(format!("set_at は Vec に使う（実際は {}）", show(&t)).into());
                };
                self.check(i, &int)?;
                self.check(x, a)?;
                self.in_range(v, i)?;
                t
            }
            // all(xs) は Async<T> の Vec をまとめた Async<Vec<T>>。待つと全部を並行に走らせ、全部が終わるまで戻らない
            Expr::Call(name, args) if name == "all" && self.lookup(name).is_none() => {
                let [a] = args.as_mut_slice() else {
                    return Err("all の引数は1個".into());
                };
                let at = self.expr(a)?;
                let el = match base(&at) {
                    V::VecT(x) => async_of(&x),
                    _ => None,
                };
                let Some((t, rest)) = el else {
                    return Err(format!("all には Async<T> の Vec を渡す（実際は {}）", show(&at)).into());
                };
                self.async_calls.insert(addr);
                async_t(V::VecT(Rc::new(t)), rest)
            }
            Expr::Call(name, args) if name == "resume" && self.lookup(name).is_none() => {
                let Some(t) = self.resume.last().cloned() else {
                    return Err("resume は handle の腕の中でだけ使える".into());
                };
                if t == prim("never") {
                    return Err("戻り値の型が never の操作は resume できない".into());
                }
                // resume() は unit で再開する
                match args.as_mut_slice() {
                    [] => want(&t, &prim("unit"), "resume")?,
                    [a] => self.check(a, &t)?,
                    _ => return Err("resume の引数は1個".into()),
                }
                prim("never")
            }
            Expr::Call(name, args) if self.lookup(name).is_some() => {
                let t = self.lookup(name).unwrap().ty.clone();
                return self.apply_type(&t, args, addr);
            }
            Expr::Call(name, args) if name == "put" => {
                if args.len() != 1 {
                    return Err("put の引数は1個".into());
                }
                let t = self.expr(&mut args[0])?;
                if async_of(&t).is_some() {
                    return Err(format!("put: {} の値は表示できない（.await を付けて待つ）", show(&t)).into());
                }
                if let Some(inst) = self.trait_inst("show", &t)? {
                    let a = args.remove(0);
                    let shown = Expr::Call("std.to_string".into(), vec![a, inst]);
                    args.push(shown);
                }
                prim("unit")
            }
            Expr::Call(name, _) if name == "Type" => return Err("Type に引数は指定できない".into()),
            Expr::Call(name, args) if self.norm.ctors.contains_key(&name) => {
                return self.ctor_call(name, args, None);
            }
            Expr::Call(name, args) if self.norm.datas.contains_key(&name) => {
                return self.data_app(name, args);
            }
            Expr::Apply(f, args) => {
                let t = self.expr(f)?;
                return self.apply_type(&t, args, addr);
            }
            Expr::Lam(params, ret, effs, body) => {
                return self.lam_infer(params, ret, effs, body, None);
            }
            Expr::Pi(binders, ret, effs, reqs) => return self.pi_type(binders, ret, effs, reqs),
            Expr::Match(s, arms) => return self.check_match(s, arms, None),
            // 書き換えられる参照。new_ref(x) で作り、read(r) で読み、write(r, x) で書く
            Expr::Call(name, args) if name == "Ref" => {
                let [a] = args.as_mut_slice() else {
                    return Err("Ref の引数は1個".into());
                };
                self.type_level(a).ctx(|e| format!("Ref の引数: {e}"))?;
                V::Type
            }
            Expr::Call(name, args) if name == "new_ref" && self.lookup(name).is_none() => {
                let [a] = args.as_mut_slice() else {
                    return Err("new_ref の引数は1個".into());
                };
                ref_t(base(&self.expr(a)?))
            }
            Expr::Call(name, args) if name == "read" && self.lookup(name).is_none() => {
                let [r] = args.as_mut_slice() else {
                    return Err("read の引数は1個".into());
                };
                let t = self.expr(r)?;
                ref_of(&t).ok_or(format!("read は Ref に使う（実際は {}）", show(&t)))?
            }
            Expr::Call(name, args) if name == "write" && self.lookup(name).is_none() => {
                let [r, x] = args.as_mut_slice() else {
                    return Err("write の引数は2個".into());
                };
                let t = self.expr(r)?;
                let a = ref_of(&t).ok_or(format!("write は Ref に使う（実際は {}）", show(&t)))?;
                self.check(x, &a)?;
                prim("unit")
            }
            Expr::Call(name, args) if name == "len" || name == "Vec" => {
                if args.len() != 1 {
                    return Err(format!("{name} の引数は1個").into());
                }
                if name == "Vec" {
                    self.type_level(&args[0]).ctx(|e| format!("Vec の引数: {e}"))?;
                    return Ok(V::Type);
                }
                match base(&self.expr(&mut args[0])?) {
                    V::VecT(_) | V::Prim("String") => int,
                    t if self.in_macro && same(&prim("syn"), &t) => int,
                    t => return Err(format!("len は Vec に使う（実際は {}）", show(&t)).into()),
                }
            }
            Expr::Call(name, args) if self.in_macro && name == "at" && args.len() == 2 => {
                let t = self.expr(&mut args[0])?;
                want(&prim("syn"), &t, "at")?;
                self.check(&mut args[1], &int)?;
                match base(&t) {
                    V::Syn(k) if k.starts_with('[') => syn_of(&k[1..k.len() - 1]),
                    V::Syn(k) => return Err(format!("at は配列に使う（実際は syn<{k}>）").into()),
                    _ => prim("syn"),
                }
            }
            Expr::Call(name, args) if self.in_macro && SYN_FNS.iter().any(|s| s.0 == name) => {
                let (_, params, ret) = SYN_FNS.iter().find(|s| s.0 == name).unwrap();
                if params.len() != args.len() {
                    return Err(format!("{name} の引数は{}個", params.len()).into());
                }
                for (p, a) in params.iter().zip(args.iter_mut()) {
                    self.check(a, &prim(p))?;
                }
                prim(ret)
            }
            Expr::Call(name, args) if name != "to_str" && !STR_FNS.iter().any(|s| s.0 == name) => {
                if !self.norm.fns.contains_key(&name) {
                    let e: Error = format!("未定義の関数 {name}").into();
                    return Err(self.suggest(e, name));
                }
                return self.call_fn(name, args, None, addr);
            }
            Expr::Assign(name, x) => {
                let en = self.lookup(name).ok_or(format!("未定義の変数 {name}"))?;
                if !en.mutable {
                    return Err(format!("変数 {name} は mut でない").into());
                }
                let t = en.ty.clone();
                self.check(x, &t).ctx(|e| format!("変数 {name}: {e}"))?;
                // 代入前の長さが分かっていれば、新しい長さを代入前の変数を使わない式にして事実にする
                let new_len = if matches!(base(&t), V::VecT(_)) {
                    self.new_len(name, x)?
                } else {
                    None
                };
                self.forget(name)?;
                if let Some((v, l)) = new_len {
                    self.add_fact(bin("==", len(v), l));
                }
                prim("unit")
            }
            Expr::If(c, then, els) => {
                want(&boolean, &self.expr(c)?, "if の条件")?;
                let cv = self.eval_value(c)?;
                self.enter_branch(&cv, true);
                let tt = self.block(then);
                self.pop();
                let Some(els) = els else {
                    // else が無い if の値は捨てる
                    if tt? == prim("never") { self.add_fact(un("!", cv)); }
                    return Ok(prim("unit"));
                };
                // 片方の分岐だけで型が決まらない時（None や []）は、もう片方の型で確かめる
                let tt = match tt {
                    Ok(t) => t,
                    Err(e) => {
                        self.enter_branch(&cv, false);
                        let et = self.expr(els);
                        self.pop();
                        let Ok(et) = et else { return Err(e) };
                        let et = base(&et);
                        self.enter_branch(&cv, true);
                        let r = self.check_block(then, &et);
                        self.pop();
                        r?;
                        return Ok(et);
                    }
                };
                self.enter_branch(&cv, false);
                let et = self.expr(els);
                self.pop();
                let et = match et {
                    Ok(t) => t,
                    Err(e) if tt != prim("never") => {
                        self.enter_branch(&cv, false);
                        let r = self.check(els, &base(&tt));
                        self.pop();
                        r.map_err(|_| e)?;
                        return Ok(base(&tt));
                    }
                    Err(e) => return Err(e),
                };
                if tt == prim("never") { et }
                else if et == prim("never") || tt == et {
                    tt
                } else {
                    let (tb, eb) = (base(&tt), base(&et));
                    want(&tb, &eb, "if の分岐")?;
                    if tb == prim("never") { eb } else { tb }
                }
            }
            // 残りは文字列の組み込み関数
            Expr::Call(name, args) => {
                let (_, params, ret) = STR_FNS.iter().find(|s| s.0 == name).unwrap();
                if params.len() != args.len() {
                    return Err(format!("{name} の引数は{}個", params.len()).into());
                }
                for (p, a) in params.iter().zip(args.iter_mut()) {
                    self.check(a, &prim(p))?;
                }
                if *ret == "Vec<String>" {
                    V::VecT(Rc::new(prim("String")))
                } else {
                    prim(ret)
                }
            }
            Expr::For(x, it, end, body) => {
                // 繰り返しの中で代入される変数についての事実は、2回目以降には成り立たない
                let mut names = Vec::new();
                assigned(it, &mut names);
                if let Some(end) = &end {
                    assigned(end, &mut names);
                }
                assigned_block(body, &mut names);
                for n in &names {
                    self.forget(n)?;
                }
                self.push();
                let r = (|| {
                    match end {
                        // for i in a..b では a <= i < b
                        Some(end) => {
                            self.check(it, &int)?;
                            self.check(end, &int)?;
                            let (a, b) = (self.eval_value(it)?, self.eval_value(end)?);
                            let id = self.norm.fresh();
                            let v = neu(Term::Var(id, x.clone()));
                            self.bind(x, int.clone(), v.clone(), false)?;
                            self.add_fact(bin("<=", a, v.clone()));
                            self.add_fact(bin("<", v, b));
                        }
                        None => {
                            let V::VecT(el) = base(&self.expr(it)?) else {
                                return Err("for は Vec か a..b に使う".into());
                            };
                            let v = self.norm.opaque();
                            self.bind(x, (*el).clone(), v, false)?;
                        }
                    }
                    self.block(body)
                })();
                self.pop();
                r?;
                for n in &names {
                    self.forget(n)?;
                }
                prim("unit")
            }
            Expr::While(c, body) => {
                want(&boolean, &self.expr(c)?, "while の条件")?;
                // 繰り返しの中で代入される変数についての事実は、2回目以降には成り立たない
                let mut names = Vec::new();
                assigned(c, &mut names);
                assigned_block(body, &mut names);
                for n in &names {
                    self.forget(n)?;
                }
                let cv = self.eval_value(c)?;
                self.enter_branch(&cv, true);
                let r = self.block(body);
                self.pop();
                r?;
                for n in &names {
                    self.forget(n)?;
                }
                // 抜けた後は条件が偽
                self.add_fact(un("!", cv));
                prim("unit")
            }
            Expr::Block(b) => self.block(b)?,
            Expr::Return(x) => {
                let ret = self.ret.clone();
                match x {
                    Some(x) => self.check(x, &ret)?,
                    None => want(&ret, &prim("unit"), "return")?,
                }
                prim("never")
            }
        })
    }

    // トップレベルの関数の呼び出し。ジェネリック引数を省いた時は、メタ変数にして引数と期待する型から求める
    fn call_fn(&mut self, name: &Name, args: &mut Vec<Expr>, expected: Option<&V>, addr: NodeId) -> R<V> {
        let f = self.norm.fns[name].clone();
        // 宣言に Async を書いた関数の呼び出しは、効果を起こさずに Async<T> の値になる（.await で待つ）。
        // 効果の変数を通して Async が入る呼び出しは、今までどおりその場で待つ
        let delayed = f.effects.iter().any(|e| matches!(e.peel(), Expr::Var(n) if n == "Async"));
        let expected_async = expected.and_then(async_of).map(|(t, _)| t);
        let expected = if delayed { expected_async.as_ref() } else { expected };
        let explicit = f.params.len() - f.implicit - f.insts;
        // with の要求は、内側のスコープで選ばれた値を引数にして渡す。
        let caps: Vec<_> = f.params[f.implicit..f.params.len() - f.insts]
            .iter().filter(|(name, _)| name.capability_trait().is_some()).map(|(name, _)| name.clone()).collect();
        if !caps.is_empty() && args.len() == explicit - caps.len() {
            for cap in caps {
                if self.lookup(&cap).is_none() {
                    return Err(format!("関数 {name} には with {} が必要", cap.capability_trait().unwrap()).into());
                }
                args.push(Expr::Var(cap));
            }
        }
        let n = args.len();
        // 明示した型引数と、補完済みの辞書引数は個数が一致する場合がある。
        let type_argument = args.first().is_some_and(|e| match e.peel() {
            Expr::Var(n) => self.lookup(n).is_some_and(|e| e.ty == V::Type)
                || PRIMS.contains(&n.as_str()) || self.norm.datas.contains_key(n)
                || self.norm.fns.get(n).is_some_and(|f| f.alias),
            Expr::Call(n, _) => n == "Vec" || n == "Ref" || self.norm.datas.contains_key(n)
                || self.norm.fns.get(n).is_some_and(|f| f.alias),
            Expr::Refine(..) | Expr::Pi(..) => true,
            _ => false,
        });
        let explicit_types = f.implicit > 0 && n == f.params.len() - f.insts && type_argument;
        let infer = f.implicit > 0 && !explicit_types && (n == explicit || n == explicit + f.insts);
        // 制約の実装を省いた呼び出し。実装は型引数が決まってから探す
        let omit = f.insts > 0 && (n == explicit || n == f.params.len() - f.insts);
        if !infer && !omit && n != f.params.len() {
            return Err(format!("関数 {name} の引数は{explicit}個").into());
        }
        // 実装を足す時に引数の並びが動かないよう、先に容量を確保する
        if omit {
            args.reserve(f.insts);
        }
        // 引数の型の細分化の条件は前の引数の値を参照できるので、値を順に積みながら計算する
        let mut fenv: Env = vec![HashMap::new()];
        let mut metas = Vec::new();
        let mut sol = HashMap::new();
        let params = if infer {
            for (p, _) in &f.params[..f.implicit] {
                let id = self.norm.fresh();
                metas.push((id, p.clone()));
                fenv[0].insert(p.clone(), neu(Term::Var(id, format!("?{p}").into())));
            }
            &f.params[f.implicit..]
        } else {
            &f.params[..]
        };
        // 操作の呼び出しでは、効果の型引数を、起こしてよい効果の型から決める
        if let (Some(eff), true) = (&f.op, infer)
            && let Some((_, Some(V::DataT(d)))) = self.avail.iter().rev().find(|(n, _)| n == eff)
        {
            for ((id, _), v) in metas.iter().zip(&d.1) {
                sol.insert(*id, v.clone());
            }
        }
        let ids: Vec<usize> = metas.iter().map(|m| m.0).collect();
        // 期待する型が分かれば、先に戻り値の型と合わせる
        if infer && let (Some(exp), Some(rt)) = (expected, &f.ret) {
            let mut env1 = fenv.clone();
            for (p, _) in params {
                let v = self.norm.opaque();
                env1[0].insert(p.clone(), v);
            }
            let rv = self.norm.eval_or_opaque(rt, &mut env1)?;
            self.unify(&rv, &base(exp), &ids, &mut sol)?;
        }
        let mut vals = Vec::new();
        for ((pn, pt), a) in params.iter().zip(args.iter_mut()) {
            let r = |e: &str| format!("関数 {name} の引数 {pn}: {e}");
            let t = self.norm.eval_or_opaque(pt, &mut fenv)?;
            let t = self.apply(&sol, &t)?;
            if ids
                .iter()
                .any(|id| !sol.contains_key(id) && mentions(&t, *id))
            {
                let open = |v: &V| {
                    ids.iter()
                        .any(|id| !sol.contains_key(id) && mentions(v, *id))
                };
                let at = if let Expr::Lam(ps, ret, effs, body) = a.peel_mut()
                    && let V::Pi(p) = base(&t)
                    && !p.doms.iter().any(open)
                    && p.reqs.is_empty()
                {
                    // 引数の型が決まっていれば、無名関数の引数の型を省ける。戻り値の型と効果は本体から求める
                    let collect = effs.is_empty() && p.effs.iter().any(open);
                    self.lam_infer(ps, ret, effs, body, Some((&p, collect)))
                        .ctx(r)?
                } else {
                    self.expr(a).ctx(r)?
                };
                if !self.unify(&t, &at, &ids, &mut sol)? {
                    return Err(r(&format!("{} が必要だが {}", show(&t), show(&at))).into());
                }
            } else {
                self.check(a, &t).ctx(r)?;
            }
            let v = self.eval_value(a)?;
            vals.push(v.clone());
            fenv[0].insert(pn.clone(), v);
        }
        // 引数から決まらない型引数（trait Convert<T> の T）は、制約の実装から決める
        if infer && omit && ids.iter().any(|id| !sol.contains_key(id)) {
            for (_, pt) in &f.params[f.params.len() - f.insts..] {
                let t = self.norm.eval_or_opaque(pt, &mut fenv.clone())?;
                let t = self.apply(&sol, &t)?;
                let open: Vec<usize> = ids.iter().copied().filter(|id| !sol.contains_key(id) && mentions(&t, *id)).collect();
                if !open.is_empty()
                    && let Ok((_, it)) = self.resolve_in(&t, 0, &open)
                {
                    self.unify(&t, &it, &ids, &mut sol)?;
                }
            }
        }
        if infer {
            let mut solved = Vec::new();
            for (id, p) in &metas {
                let is_eff = f
                    .params
                    .iter()
                    .any(|(n, t)| n == p && matches!(t.peel(), Expr::Var(k) if k == "Effect"));
                let v = match sol.get(id) {
                    Some(v) => v.clone(),
                    // 効果の変数が決まらなければ、効果は無い
                    None if is_eff => V::Arr(Rc::new(vec![])),
                    None => {
                        return Err(format!(
                            "関数 {name} の型引数 {p} を推論できない。{name}<...>(...) で明示する"
                        )
                        .into());
                    }
                };
                let v = self.apply(&sol, &v)?;
                fenv[0].insert(p.clone(), v.clone());
                solved.push(v);
            }
            solved.extend(vals);
            vals = solved;
        }
        if omit {
            let mut add = Vec::new();
            for (pn, pt) in &f.params[f.params.len() - f.insts..] {
                let t = self.norm.eval_or_opaque(pt, &mut fenv)?;
                let t = self.apply(&sol, &t)?;
                add.push(self.resolve_inst(&t, 0)?);
                let v = self.norm.opaque();
                fenv[0].insert(pn.clone(), v.clone());
                vals.push(v);
            }
            args.extend(add);
        }
        if let Some(eff) = &f.op {
            let args = f.params[..f.implicit]
                .iter()
                .map(|(p, _)| fenv[0][p].clone())
                .collect();
            self.use_effect(V::DataT(Rc::new((eff.clone(), args))))
                .map_err(|er| er.help(format!("{name} は効果 {eff} の操作")))?;
        }
        // 効果の変数に Async が入れば、JavaScript では多相な関数の async 版を呼んで待つ
        let poly = f.params[..f.implicit]
            .iter()
            .any(|(_, t)| matches!(t.peel(), Expr::Var(k) if k == "Effect"));
        let mut rest = Vec::new();
        for ef in &f.effects {
            for v in self.effect_inst(ef, &mut fenv)? {
                let k = eff_key(&v);
                if poly && k == "Async" {
                    self.async_inst.insert(addr);
                    self.awaits.insert(addr);
                }
                if k.is_effect_parameter() {
                    self.var_awaits.insert(addr);
                }
                // Async<T> の値を待つ時に起こす
                if delayed {
                    if k != "Async" {
                        rest.push(v);
                    }
                    continue;
                }
                let s = show(&v);
                let origin = op_site(&self.norm.fns, &f, &k);
                self.use_effect(v).map_err(|mut er| {
                    if er.origin.is_none() {
                        if let Some(span) = origin {
                            er.origin = Some((span.start.offset, span.end.offset - span.start.offset));
                            er.origin_source = Some(span.source);
                        }
                    }
                    er.help(format!("関数 {name} の型が ! {s} を持つ"))
                })?;
            }
        }
        let ret = match &f.ret {
            Some(t) => self.norm.eval_or_opaque(t, &mut fenv)?,
            None => prim("unit"),
        };
        let ret = self.apply(&sol, &ret)?;
        if delayed {
            self.async_calls.insert(addr);
            let rest = rest.iter().map(|v| self.apply(&sol, v)).collect::<R<Vec<_>>>()?;
            return Ok(async_t(ret, rest));
        }
        Ok(ret)
    }

    // 未定義の名前のエラーに、綴りの近い名前を提案する
    fn suggest(&self, e: Error, n: &str) -> Error {
        let names = self
            .scopes
            .iter()
            .flat_map(|s| s.vars.keys().map(|k| k.as_str()))
            .chain(self.norm.fns.keys().map(|k| k.as_str()))
            .chain(STR_FNS.iter().map(|s| s.0))
            .chain([
                "put", "len", "push", "set_at", "to_str", "new_ref", "read", "write", "all",
                "resume",
            ])
            .map(|k| k.to_string());
        let best = names
            .map(|k| (edit_distance(n, &k), k))
            .filter(|(d, _)| *d <= 2 && *d < n.chars().count())
            .min();
        match best {
            Some((_, k)) => e.help(format!("{k} のことか")),
            None => e,
        }
    }

    fn fn_type(&mut self, f: &Fn) -> R<V> {
        if f.op.is_some() {
            return Err(format!(
                "効果の操作 {} は値として使えない（呼び出しだけができる）",
                f.name
            )
            .into());
        }
        let ret = f.ret.clone().unwrap_or(Expr::Var("unit".into()));
        let mut env: Env = vec![HashMap::new()];
        // with の要求を持つ関数は、要求を型に載せる。値として呼ぶ時は、呼ぶ側のスコープの値と実装を引数の後ろに受け取る
        // ponytail: 要求の集合を型引数にする（with R で多相にする）は未対応。高階関数（map など）は要求を転送できず、要求の並びが完全に一致する関数だけ渡せる。転送するには要求の並びを型引数にし、呼び出しごとに値の並びを作る
        let captured = &f.params[..f.implicit];
        let (params, reqs) = if captured.iter().any(|(n, _)| n.capability_type_trait().is_some()) {
            if !captured.iter().all(|(n, _)| n.capability_type_trait().is_some()) {
                return Err(format!("総称関数 {} は値として使えない（with の要求だけなら使える）", f.name).into());
            }
            let body = &f.params[f.implicit..f.params.len() - f.insts];
            let explicit: Vec<(Name, Expr)> = body.iter().filter(|(n, _)| n.capability_trait().is_none()).cloned().collect();
            let reqs: Vec<Name> = body.iter().filter_map(|(n, _)| n.capability_trait().cloned()).collect();
            if reqs.len() != f.insts {
                return Err(format!("制約つきの関数 {} は値として使えない", f.name).into());
            }
            (explicit, reqs)
        } else {
            (f.params.clone(), Vec::new())
        };
        match self.norm.eval_pi(&params, &ret, &f.effects, &reqs, &mut env) {
            Ok(v) => Ok(v),
            Err(Halt::Stuck) => Ok(self.norm.opaque()),
            Err(Halt::Err(e)) => Err(e),
        }
    }

    // 関数型 t の値を args で呼んだ時の型
    // e は呼び出しの式。Async の効果を持つ関数なら、JavaScript で待つ呼び出しとして記録する
    fn apply_type(&mut self, t: &V, args: &mut Vec<Expr>, addr: NodeId) -> R<V> {
        let V::Pi(p) = base(t) else {
            return Err(format!("関数でない値（{}）を呼んでいる", show(t)).into());
        };
        if p.doms.len() != args.len() {
            return Err(format!("引数は{}個", p.doms.len()).into());
        }
        for (i, a) in args.iter_mut().enumerate() {
            self.check(a, &p.doms[i])
                .ctx(|e| format!("{} 番目の引数: {e}", i + 1))?;
        }
        // with の要求は、呼ぶ側のスコープの値と実装を引数の後ろに足す（関数の宣言と同じ並び）
        let mut insts = Vec::new();
        for tr in &p.reqs {
            let cap = Name::capability(tr.clone(), false);
            let Some(entry) = self.lookup(&cap) else {
                return Err(format!("この関数の値を呼ぶには with {tr} が必要").into());
            };
            let ty = entry.ty.clone();
            insts.push(self.resolve_inst(&V::DataT(Rc::new((tr.clone(), vec![ty]))), 0)?);
            args.push(Expr::Var(cap));
        }
        args.extend(insts);
        // 関数型に直接 Async を書いた値の呼び出しは Async<T> の値になる（効果の変数に入った Async は除く）
        let delayed = p.effs.iter().any(|e| matches!(e, V::DataT(d) if d.0 == "Async"));
        let mut rest = Vec::new();
        for eff in &p.effs {
            for eff in flat(&[eff.clone()]) {
                if eff_key(&eff) == "Async" {
                    self.awaits.insert(addr);
                }
                if eff_key(&eff).is_effect_parameter() {
                    self.var_awaits.insert(addr);
                }
                if delayed {
                    if eff_key(&eff) != "Async" {
                        rest.push(eff);
                    }
                    continue;
                }
                let s = show(&eff);
                self.use_effect(eff)
                    .map_err(|er| er.help(format!("呼んだ関数型の値が ! {s} を持つ")))?;
            }
        }
        if delayed {
            self.async_calls.insert(addr);
            return Ok(async_t(p.cod.clone(), rest));
        }
        Ok(p.cod.clone())
    }

    // 関数の本体を、戻り値の型 rt で検査する
    // avail は本体で起こしてよい効果
    fn fn_body(&mut self, body: &mut Block, rt: Option<V>, avail: Vec<(Name, Option<V>)>) -> R<V> {
        if let Some(V::Type) = rt.as_ref().map(base) {
            return Err("関数は型を返せない。型を計算するなら type で定義する".into());
        }
        let saved = self.ret.clone();
        let saved_eff = (
            std::mem::replace(&mut self.avail, avail),
            std::mem::take(&mut self.resume),
        );
        let r = match rt {
            Some(rt) => {
                self.ret = rt.clone();
                if rt == prim("unit") {
                    self.block(body).and_then(|t| no_drop(&t)).map(|_| rt)
                } else {
                    self.check_block(body, &rt).map(|_| rt)
                }
            }
            None => {
                // 戻り値の型が無いと return の型が分からない
                self.ret = self.norm.opaque();
                self.block(body)
            }
        };
        self.ret = saved;
        (self.avail, self.resume) = saved_eff;
        if let Ok(V::Type) = r.as_ref().map(base) {
            return Err("関数は型を返せない。型を計算するなら type で定義する".into());
        }
        r
    }

    // 無名関数の型を、書かれた引数の型から求める
    // hint は引数の型だけが分かっている関数型。true なら効果も本体から集める
    fn lam_infer(
        &mut self,
        params: &[(Name, Option<Expr>)],
        ret: &Option<Box<Expr>>,
        effs: &[Expr],
        body: &mut Block,
        hint: Option<(&PiV, bool)>,
    ) -> R<V> {
        if let Some((p, _)) = hint
            && p.doms.len() != params.len()
        {
            return Err(format!("引数は{}個", p.doms.len()).into());
        }
        self.push();
        let r = (|| {
            let mut doms = Vec::new();
            for (i, (n, t)) in params.iter().enumerate() {
                // 引数の型の分かる関数型の、i 番目の引数の型
                let d = hint.map(|(p, _)| p.doms[i].clone());
                let ty = match (t, d) {
                    (Some(t), d) => {
                        let ty = self.eval_type(t)?;
                        if let Some(d) = d {
                            want(&ty, &d, &format!("引数 {n}"))?;
                        }
                        ty
                    }
                    (None, Some(d)) => d,
                    (None, None) => {
                        return Err(format!(
                            "無名関数の引数 {n} の型を書く（期待する型が無いので推論できない）"
                        )
                        .into());
                    }
                };
                let id = self.norm.fresh();
                self.bind(n, ty.clone(), neu(Term::Var(id, n.clone())), false)?;
                doms.push(ty);
            }
            let rt = match ret {
                Some(t) => Some(self.eval_type(t)?),
                None => None,
            };
            // 書いた効果があればそれだけを起こせる。無ければ、効果の変数を持つ関数型に渡す時は本体で起きた効果を集め、
            // それ以外の関数型に渡す時はその効果を起こせる。期待する型の無い無名関数も本体で起きた効果を集める
            let mut es = Vec::new();
            let mut avail = Vec::new();
            for e in effs {
                let mut env = self.val_env();
                for v in self.effect_inst(e, &mut env)? {
                    self.note_body(&v, body);
                    avail.push((eff_key(&v), Some(v.clone())));
                    es.push(v);
                }
            }
            let collect = matches!(hint, Some((_, true))) || (hint.is_none() && effs.is_empty());
            if let (Some((p, false)), true) = (hint, effs.is_empty()) {
                for e in &p.effs {
                    for e in flat(&[e.clone()]) {
                        self.note_body(&e, body);
                        avail.push((eff_key(&e), Some(e.clone())));
                        es.push(e);
                    }
                }
            }
            if collect {
                avail.push((Name::from("*"), None));
            }
            let saved = std::mem::replace(&mut self.collected, collect.then(Vec::new));
            let cod = self.fn_body(body, rt, avail);
            let found = std::mem::replace(&mut self.collected, saved);
            let cod = cod?;
            for e in found.unwrap_or_default() {
                self.note_body(&e, body);
                es.push(e);
            }
            Ok(V::Pi(Rc::new(PiV { doms, cod, effs: es, reqs: Vec::new() })))
        })();
        self.pop();
        r
    }

    // 要求を持つ期待する関数型で無名関数を検査する。要求の値と実装は、無名関数の引数の後ろに足す（本体では名前で使える）
    fn lam_with_reqs(
        &mut self,
        params: &mut Vec<(Name, Option<Expr>)>,
        ret: &Option<Box<Expr>>,
        body: &mut Block,
        p: &PiV,
    ) -> R<()> {
        self.push();
        let hidden = self.bind_reqs(&p.reqs);
        let r = match &hidden {
            Ok(_) => self.lam_check(params, ret, body, p),
            Err(_) => Ok(()),
        };
        self.pop();
        let hidden = hidden?;
        r?;
        params.extend(hidden.iter().map(|(cap, _)| (cap.clone(), None)));
        params.extend(hidden.iter().map(|(_, inst)| (inst.clone(), None)));
        Ok(())
    }

    // 要求ごとに、値と実装の名前を今のスコープに束縛する。値の型は、呼ぶ側が決める未知の型（変数）
    fn bind_reqs(&mut self, reqs: &[Name]) -> R<Vec<(Name, Name)>> {
        let mut hidden = Vec::new();
        for tr in reqs {
            let id = self.norm.fresh();
            let ty = neu(Term::Var(id, Name::capability(tr.clone(), true)));
            let cap = Name::capability(tr.clone(), false);
            let value = self.norm.fresh();
            self.bind(&cap, ty.clone(), neu(Term::Var(value, cap.clone())), false)?;
            let inst = Name::instance(self.norm.fresh());
            let inst_value = self.norm.fresh();
            self.bind(&inst, V::DataT(Rc::new((tr.clone(), vec![ty]))), neu(Term::Var(inst_value, inst.clone())), false)?;
            hidden.push((cap, inst));
        }
        Ok(hidden)
    }

    // 無名関数を、期待する関数型 p で検査する。引数の型は省いてよい
    fn lam_check(
        &mut self,
        params: &[(Name, Option<Expr>)],
        ret: &Option<Box<Expr>>,
        body: &mut Block,
        p: &PiV,
    ) -> R<()> {
        if params.len() != p.doms.len() {
            return Err(format!("引数は{}個", p.doms.len()).into());
        }
        self.push();
        let r = (|| {
            for (i, (n, t)) in params.iter().enumerate() {
                let d = p.doms[i].clone();
                if let Some(t) = t {
                    let tt = self.eval_type(t)?;
                    want(&tt, &d, &format!("引数 {n}"))?;
                }
                let id = self.norm.fresh();
                self.bind(n, d, neu(Term::Var(id, n.clone())), false)?;
            }
            if let Some(rt) = ret {
                let rt = self.eval_type(rt)?;
                want(&p.cod, &rt, "戻り値")?;
            }
            let mut avail = Vec::new();
            for e in &p.effs {
                for e in flat(&[e.clone()]) {
                    self.note_body(&e, body);
                    avail.push((eff_key(&e), Some(e)));
                }
            }
            self.fn_body(body, Some(p.cod.clone()), avail).map(|_| ())
        })();
        self.pop();
        r
    }

    // 関数型 (x: A, ...) -> B の型を検査する
    fn pi_type(&mut self, binders: &[(Name, Expr)], ret: &Expr, effs: &[Expr], _reqs: &[Name]) -> R<V> {
        self.push();
        let r = (|| {
            for (n, t) in binders {
                self.type_level(t)?;
                let ty = self.eval_value(t)?;
                let id = self.norm.fresh();
                self.bind(n, ty, neu(Term::Var(id, n.clone())), false)?;
            }
            for e in effs {
                let mut env = self.val_env();
                self.effect_inst(e, &mut env)?;
            }
            self.type_level(ret)?;
            Ok(V::Type)
        })();
        self.pop();
        r
    }

    // 式が型かを検査する
    fn type_level(&mut self, t: &Expr) -> R<()> {
        let mut t = t.clone();
        match base(&self.expr(&mut t)?) {
            V::Type => Ok(()),
            other => Err(format!("型が必要だが {}", show(&other)).into()),
        }
    }

    // data の型引数とフィールドの型を検査する
    fn data_level(&mut self, d: &Data) -> R<()> {
        self.scopes = vec![Scope::default()];
        for (n, t) in &d.params {
            let ty = self
                .eval_type(t)
                .ctx(|e| format!("型 {} の引数 {n}: {e}", d.name))?;
            let id = self.norm.fresh();
            self.bind(n, ty, neu(Term::Var(id, n.clone())), false)?;
        }
        for c in &d.ctors {
            let r = |e: &str| format!("構築子 {}: {e}", c.name);
            self.push();
            for (n, t) in &c.fields {
                self.type_level(t).ctx(r)?;
                let ty = self.eval_value(t)?;
                let id = self.norm.fresh();
                self.bind(n, ty, neu(Term::Var(id, n.clone())), false)?;
            }
            self.pop();
        }
        Ok(())
    }

    // 型名に引数を渡して型を作る。引数の型の細分化の条件は前の引数の値を参照できる
    fn data_app(&mut self, name: &Name, args: &mut [Expr]) -> R<V> {
        let d = self.norm.datas[name].clone();
        if d.params.len() != args.len() {
            return Err(format!("型 {name} の引数は{}個", d.params.len()).into());
        }
        let mut env: Env = vec![HashMap::new()];
        for ((pn, pt), a) in d.params.iter().zip(args) {
            let t = self.norm.eval_or_opaque(pt, &mut env)?;
            self.check(a, &t)
                .ctx(|e| format!("型 {name} の引数 {pn}: {e}"))?;
            let v = self.eval_value(a)?;
            env[0].insert(pn.clone(), v);
        }
        Ok(V::Type)
    }

    fn ctor_call(&mut self, name: &Name, args: &mut [Expr], expected: Option<&V>) -> R<V> {
        let (d, c) = self.norm.ctors[name].clone();
        if c.fields.len() != args.len() {
            return Err(format!("構築子 {name} の引数は{}個", c.fields.len()).into());
        }
        let mut env: Env = vec![HashMap::new()];
        let mut metas = Vec::new();
        for (p, _) in &d.params {
            let id = self.norm.fresh();
            metas.push((id, p.clone()));
            env[0].insert(p.clone(), neu(Term::Var(id, format!("?{p}").into())));
        }
        let mut sol = HashMap::new();
        if let Some(exp) = expected {
            let ret1 = V::DataT(Rc::new((d.name.clone(), d.params.iter().map(|(p, _)| env[0][p].clone()).collect())));
            let ids: Vec<usize> = metas.iter().map(|m| m.0).collect();
            if !self.unify(&ret1, &base(exp), &ids, &mut sol)? {
                return Err(format!(
                    "{} が必要だが、{name} の型は {}",
                    show(exp),
                    show(&self.apply(&sol, &ret1)?)
                )
                .into());
            }
        }
        let ids: Vec<usize> = metas.iter().map(|m| m.0).collect();
        for ((f, ft), a) in c.fields.iter().zip(args) {
            let r = |e: &str| format!("構築子 {name} の引数 {f}: {e}");
            let t = self.norm.eval_or_opaque(ft, &mut env)?;
            let t = self.apply(&sol, &t)?;
            if ids
                .iter()
                .any(|id| !sol.contains_key(id) && mentions(&t, *id))
            {
                let at = self.expr(a).ctx(r)?;
                if !self.unify(&t, &at, &ids, &mut sol)? {
                    return Err(r(&format!("{} が必要だが {}", show(&t), show(&at))).into());
                }
            } else {
                self.check(a, &t).ctx(r)?;
            }
            let v = self.eval_value(a)?;
            env[0].insert(f.clone(), v);
        }
        let ret = V::DataT(Rc::new((d.name.clone(), d.params.iter().map(|(p, _)| env[0][p].clone()).collect())));
        let ret = self.apply(&sol, &ret)?;
        if let Some((_, p)) = metas.iter().find(|(id, _)| mentions(&ret, *id)) {
            return Err(format!("構築子 {name} の型引数 {p} を推論できない。型注釈を書く").into());
        }
        Ok(ret)
    }

    // match の各腕と網羅性を検査する
    fn check_match(&mut self, s: &mut Expr, arms: &mut [Arm], expected: Option<&V>) -> R<V> {
        let st = base(&self.expr(s)?);
        let V::DataT(dt) = &st else {
            return Err(format!("match は data 型の値に使う（実際は {}）", show(&st)).into());
        };
        let Some(d) = self.norm.datas.get(&dt.0).cloned() else {
            return Err(format!("match は data 型の値に使う（実際は {}）", show(&st)).into());
        };
        let mut covered = Vec::new();
        let mut wildcard = false;
        let mut result: Option<V> = None;
        for arm in arms {
            let r = |e: &str| format!("{} の場合: {e}", arm.ctor);
            let at = |e: Error| at_opt(e, arm.span);
            self.push();
            let exp = expected.cloned();
            if arm.ctor == "_" {
                wildcard = true;
            } else {
                let Some((dd, c)) = self.norm.ctors.get(&arm.ctor).cloned() else {
                    self.pop();
                    return Err(at(format!("不明な構築子 {}", arm.ctor).into()));
                };
                if dd.name != d.name || arm.binds.len() != c.fields.len() {
                    self.pop();
                    return Err(at(format!(
                        "{} は {} の構築子でないか、フィールドの数が違う",
                        arm.ctor, d.name
                    )
                    .into()));
                }
                covered.push(c.name.clone());
                let mut env: Env = vec![d.params.iter().map(|(p, _)| p.clone()).zip(dt.1.iter().cloned()).collect()];
                for ((field, field_type), bind) in c.fields.iter().zip(&arm.binds) {
                    let ty = self.norm.eval_or_opaque(field_type, &mut env).ctx(r).map_err(at)?;
                    let id = self.norm.fresh();
                    let value = neu(Term::Var(id, bind.clone()));
                    env[0].insert(field.clone(), value.clone());
                    if bind != "_" { self.bind(bind, ty, value, false)?; }
                }
            }
            let t = match &exp {
                Some(e) => self.check(&mut arm.body, e).map(|_| e.clone()),
                None => self.expr(&mut arm.body),
            };
            self.pop();
            let t = t.ctx(r).map_err(at)?;
            if expected.is_some() {
                continue;
            }
            result = Some(match result {
                None => t,
                Some(prev) if prev == prim("never") => t,
                Some(prev) if t == prim("never") || prev == t => prev,
                Some(prev) => {
                    want(&base(&prev), &t, &r("場合ごとの型")).map_err(at)?;
                    base(&prev)
                }
            });
        }
        if !wildcard {
            for c in &d.ctors {
                if covered.contains(&c.name) {
                    continue;
                }
                return Err(format!("match に {} の場合が無い", c.name).into());
            }
        }
        Ok(expected.cloned().or(result).unwrap_or(prim("never")))
    }

    // 構造体の値 x のフィールド f の型。フィールドの型の細分化の条件は前のフィールドの値を参照できる
    fn struct_field(&mut self, d: &(Name, Vec<V>), x: &Expr, f: &Name) -> R<V> {
        let Some(data) = self.norm.datas.get(&d.0).cloned() else {
            return Err(format!("型 {} は構造体でないのでフィールドは無い", d.0).into());
        };
        let [c] = data.ctors.as_slice() else {
            return Err(format!("型 {} は構造体でないのでフィールドは無い", d.0).into());
        };
        if c.name.structure_owner().is_none() {
            return Err(format!("型 {} は構造体でないのでフィールドは無い", d.0).into());
        }
        let xv = self.eval_value(x)?;
        let mut env: Env = vec![
            data.params
                .iter()
                .map(|p| p.0.clone())
                .zip(d.1.iter().cloned())
                .collect(),
        ];
        for (i, (n, ft)) in c.fields.iter().enumerate() {
            let t = self.norm.eval_or_opaque(ft, &mut env)?;
            if n == f {
                return Ok(t);
            }
            let v = match &xv {
                V::Data(dv) => dv.1[i].clone(),
                _ => self.norm.opaque(),
            };
            env[0].insert(n.clone(), v);
        }
        Err(format!("構造体 {} にフィールド {f} は無い", d.0).into())
    }

    // syn<種類> のフィールド f の型。種類が syntax 規則なら、そのフィールドがあることを確かめる
    fn syn_field(&self, t: &V, f: &str) -> R<V> {
        let V::Syn(k) = t else { return Ok(prim("syn")) };
        if k.starts_with('[') {
            return Err(format!("syn<{k}> は配列なので、要素は at で取り出す").into());
        }
        match self.rules.get(&**k) {
            Some(fs) => match fs.iter().find(|x| x.0 == f) {
                Some((_, fk)) => Ok(syn_of(fk)),
                None => Err(format!(
                    "syntax {k} にフィールド {f} は無い（あるのは {}）",
                    fs.iter()
                        .map(|x| x.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .into()),
            },
            None => Err(format!("syn<{k}> にフィールドは無い").into()),
        }
    }

    fn check_q(&mut self, q: &mut [Q]) -> R<()> {
        for x in q {
            match x {
                Q::Tok(_) | Q::Field(_) => {}
                Q::Var { path, .. } => {
                    let t = self
                        .lookup(&Name::from(&path[0]))
                        .ok_or(format!("未定義の変数 {}", path[0]))?
                        .ty
                        .clone();
                    if path.len() > 1 {
                        want(&prim("syn"), &t, &format!("${}", path.join(".")))?;
                        // $( )* の中では配列の要素を指すので、配列の括弧を外して辿る
                        let mut t = base(&t);
                        for f in &path[1..] {
                            if let V::Syn(k) = &t {
                                let k = k.trim_start_matches('[').trim_end_matches(']').to_string();
                                t = self.syn_field(&V::Syn(k.into()), f)?;
                            }
                        }
                    }
                }
                Q::Splice(_, e) => {
                    let t = self.expr(e)?;
                    if t == prim("unit") || t == prim("never") {
                        return Err("${ } の値を差し込めない".into());
                    }
                }
                Q::Rep(inner, _) => {
                    // 繰り返しの中では、配列の束縛はその要素を指す
                    let mut names = Vec::new();
                    q_names(inner, &mut names);
                    self.push();
                    for n in names {
                        if let Some(V::Syn(k)) = self.lookup(&n).map(|e| base(&e.ty))
                            && k.starts_with('[')
                        {
                            let v = self.norm.opaque();
                            self.bind(&n, syn_of(&k[1..k.len() - 1]), v, false)?;
                        }
                    }
                    let r = self.check_q(inner);
                    self.pop();
                    r?
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::parse;

    fn check(src: &str) -> crate::error::R<()> {
        let mut prog = parse(src)?;
        super::check(&mut prog).map(|_| ())
    }

    #[test]
    fn rejects_value_type_parameters_and_indexed_constructors() {
        for source in ["type T<b: bool> = int", "enum Eq<A: Type, x: A> { Refl }", "enum Box<A> { MkBox(x: A) -> Box<A> }", "enum Box { MkBox(x: Type) }", "struct Box { x: Type }", "fn id(A: Type) {}", "type T = if true { int } else { String }"] {
            assert!(check(source).is_err(), "{source}");
        }
        check("type Items<A> = Vec<A>\nfn main() { let xs: Items<int> = [1] }").unwrap();
    }

    #[test]
    fn rejects_dependent_type_shapes_but_keeps_refinement_conditions() {
        for src in [
            "fn f(n: int) -> Vec<n> { [] }\nfn main() {}",
            "fn f(n: int, x: Vec<n>) {}\nfn main() {}",
            "fn f(g: (n: int) -> Vec<n>) {}\nfn main() {}",
            "struct S { n: int, xs: Vec<n> }\nfn main() {}",
        ] {
            assert!(check(src).is_err(), "{src}");
        }
        check("fn f(n: int, x: int[i -> i < n]) -> int[r -> r < n] { x }\nfn main() { put(f(3, 2)) }").unwrap();
    }

    #[test]
    fn recursive_functions_and_negative_types_are_allowed() {
        check("enum Bad { MkBad(f: (Bad) -> int) }\nfn even(n: int) -> bool { if n == 0 { true } else { odd(n - 1) } }\nfn odd(n: int) -> bool { if n == 0 { false } else { even(n - 1) } }\nfn main() { put(even(4)) }").unwrap();
        check("fn f(n: int) -> int { let g = f\n g(n) }\nfn main() {}").unwrap();
    }

    #[test]
    fn runtime_calls_are_opaque_to_refinement_proofs() {
        let source = "fn items() -> Vec<int> { [1] }\nfn main() { let xs = items()\n put(xs[0]) }";
        assert!(check(source).unwrap_err().to_string().contains("添字の範囲"));
        check(&source.replace("put(xs[0])", "if len(xs) > 0 { put(xs[0]) }")).unwrap();
    }

    #[test]
    fn closure() {
        let src = include_str!("../../tests/programs/basics/closure.hb");
        check(src).unwrap();
        for (from, to, msg) in [
            ("put(twice(inc)(3))", "put(twice(add)(3))", "が必要だが"),
            (
                "let inc = fn(x: int) -> int { x + 1 }",
                "let inc = fn(x) { x + 1 }",
                "型を書く",
            ),
            ("put(f(2, 3))", "put(f(2))", "引数は2個"),
            ("put(adder(10)(5))", "put(inc(10)(5))", "関数でない値"),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
        // ジェネリック引数: 推論できなければ明示を求める。期待する型からも推論する
        let g = "enum Opt<A> { Non, Som(x: A) }\nfn non<A>() -> Opt<A> { Non }\nfn id<A>(x: A) -> A { x }\n";
        check(&format!(
            "{g}fn main() {{ let x: Opt<int> = non()\n let y = non<int>()\n let z: int = id(3) }}"
        ))
        .unwrap();
        for (body, msg) in [
            ("let x = non()", "推論できない"),
            ("let y: int = id<int>('s')", "int が必要だが String"),
            ("let z: String = id(3)", "String が必要だが int"),
            ("let w = id<int, int>(3)", "引数は1個"),
        ] {
            let e = check(&format!("{g}fn main() {{ {body} }}")).unwrap_err();
            assert!(e.contains(msg), "{body} => {e}");
        }
    }

    #[test]
    fn structs() {
        let src = include_str!("../../tests/programs/types/struct.hb");
        check(src).unwrap();
        for (from, to, msg) in [
            (
                "Point { x: 1, y: 2 }",
                "Point { x: 1 }",
                "フィールド y が無い",
            ),
            (
                "Point { x: 1, y: 2 }",
                "Point { x: 1, y: 2, z: 3 }",
                "フィールド z は無い",
            ),
            (
                "Point { x: 1, y: 2 }",
                "Point { x: 1, y: true }",
                "int が必要だが bool",
            ),
            ("Size { w: 3, h: 4 }", "Size { w: 0, h: 4 }", "証明できない"),
            ("put(q.x + q.y)", "put(q.z)", "フィールド z は無い"),
            ("put(pr.snd)", "put(pr.snd + 1)", "String が必要だが int"),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
    }

    #[test]
    fn strings_and_for() {
        let src = include_str!("../../tests/programs/basics/strings.hb");
        check(src).unwrap();
        for (from, to, msg) in [
            (
                "for i in 0..len(parts) {",
                "for i in 0..len(parts) + 1 {",
                "添字の範囲",
            ),
            (
                "len(parts) > 2 && parts[2]",
                "len(parts) > 2 || parts[2]",
                "添字の範囲",
            ),
            (
                "'Hello, ' + name",
                "'Hello, ' + 1",
                "String が必要だが int",
            ),
            ("for p in parts {", "for p in 3 {", "Vec か a..b"),
            (
                "slice('hibana', 0, 3)",
                "slice('hibana', '0', 3)",
                "int が必要だが String",
            ),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
    }

    #[test]
    fn refinement() {
        let src = r#"
type Pos = int[x -> x > 0]
type NumVec<T: Type> = Vec<T>
fn get(v: Vec<int>, i: int[i -> i >= 0]) -> int {
    if i < len(v) { v[i] } else { 0 }
}
fn sum(v: NumVec<Pos>) -> int {
    let mut s = 0
    let mut i: int[x -> x >= 0] = 0
    while i < len(v) {
        s = s + v[i]
        i = i + 1
    }
    s
}
fn add(x: Pos, y: Pos) -> int[r -> r > 1] { x + y }
fn main() {
    let p: Pos = 3
    let v: NumVec<int[x -> x > 0]> = [1, 2, p]
    put(sum(v) + get(v, 1) + v[2])
}
"#;
        check(src).unwrap();
        for (from, to, msg) in [
            ("let p: Pos = 3", "let p: Pos = 0", "証明できない"),
            (
                "-> int[r -> r > 1] { x + y }",
                "-> int[r -> r > 2] { x + y }",
                "証明できない",
            ),
            ("[1, 2, p]", "[1, -2, p]", "証明できない"),
            ("+ v[2]", "+ v[3]", "添字の範囲"),
            ("get(v, 1)", "get(v, 0 - 1)", "証明できない"),
            (
                "if i < len(v) { v[i] }",
                "if i <= len(v) { v[i] }",
                "添字の範囲",
            ),
            (
                "i = i + 1\n",
                "i = i + 1\n        s = s + v[i]\n",
                "添字の範囲",
            ),
        ] {
            let e = check(&src.replace(from, to)).unwrap_err();
            assert!(e.contains(msg), "{to} => {e}");
        }
    }
}
