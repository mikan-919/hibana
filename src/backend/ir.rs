use crate::parser::Name;
// 型検査後の構文木から、WASM とネイティブの両方へ出す構造化した中間表現を作る。
// 制御フローは WASM と同じ入れ子の形（if・while・return）で持ち、goto を持たない。
// 値はすべて V（型の番号 tag と中身 payload の 2 つの 64 ビット整数）で表す。番号は runtime.hb の先頭を参照。
// 文字列の連結・比較・表示などはランタイム（runtime.hb）の関数を呼ぶ。ランタイムも同じ変換で IR にする。
// with（Handle）は、中断の印を置いて呼び出しの後で調べ、1 段ずつ戻って抜ける（Lower::checked）。腕の値は #resume で呼んだ所へ返し、exit で with を抜ける
// mut の変数が所有する Vec への push・set_at はその場で書き換える（runtime.hb の __push_own・__set_at_own）
// 利用者の関数には GC のシャドースタックを付ける（Ctx::shadow。仕組みは runtime.hb の先頭）
use crate::error::R;
use crate::parser::{self, Expr as Ast, Stmt as AstStmt};
use std::collections::{HashMap, HashSet};
use std::fmt;

pub const UNIT: i64 = 0;
pub const INT: i64 = 1;
pub const BOOL: i64 = 2;
pub const STR: i64 = 3;
pub const ARR: i64 = 4;
pub const DATA: i64 = 5;
pub const FN: i64 = 6;
pub const TY: i64 = 7;
pub const NUM: i64 = 9;
// with の記録の並び（runtime.hb の __handle_push）と Promise。ランタイムの中と非同期の変換だけで使う
pub const HANDLERS: i64 = 10;
pub const PROMISE: i64 = 11;

const RUNTIME: &str = include_str!("runtime.hb");

mod coroutine;

// runtime.hb の __static の大きさ
pub(crate) const STATIC_BYTES: usize = 184;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub(crate) functions: Vec<Function>,
    pub(crate) imports: Vec<Import>,
    // 公開関数と外部関数の境界の型（引数, 戻り値）。Vec と構造体を渡す接続コードが使う
    pub(crate) boundary: HashMap<String, (Vec<BoundaryType>, BoundaryType)>,
}

/// WASM の境界で渡す値の形。Vec と構造体は runtime.hb の __val_in・__val_out のバイト列で渡す
#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryType {
    Int,
    Number,
    Bool,
    Str,
    Unit,
    Vec(Box<BoundaryType>),
    // 構築子の番号（IR の構築子の並び）と、宣言の順のフィールド
    Struct(i64, Vec<(String, BoundaryType)>),
}

impl BoundaryType {
    // 接続コードに渡す JSON。基本の型は名前、Vec は {"vec": 要素}、構造体は {"ctor": 番号, "fields": [[名前, 型], ...]}
    pub fn json(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Self::Int => json!("int"),
            Self::Number => json!("number"),
            Self::Bool => json!("bool"),
            Self::Str => json!("String"),
            Self::Unit => json!("unit"),
            Self::Vec(x) => json!({ "vec": x.json() }),
            Self::Struct(k, fields) => json!({ "ctor": k, "fields": fields.iter().map(|(n, t)| json!([n, t.json()])).collect::<Vec<_>>() }),
        }
    }

    fn value_type(&self) -> ValueType {
        match self {
            Self::Int => ValueType::Int,
            Self::Number => ValueType::Number,
            Self::Bool => ValueType::Bool,
            Self::Str => ValueType::Str,
            Self::Unit => ValueType::Unit,
            Self::Vec(_) | Self::Struct(..) => ValueType::Value,
        }
    }
}

/// WASM の境界で渡せる型。それ以外は Other
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    Int,
    Number,
    Bool,
    Str,
    Unit,
    // Vec と構造体。中身は Program::boundary の型で決まる
    Value,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub(crate) name: String,
    pub(crate) param_types: Vec<ValueType>,
    pub(crate) result_type: ValueType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub(crate) name: String,
    // 引数の数。無名関数とその入口は、先頭に関数の値そのもの（写し取った変数を持つ）を受け取る
    pub(crate) params: u32,
    pub(crate) param_types: Vec<ValueType>,
    pub(crate) result_type: ValueType,
    // 公開関数の入口で仕事の並びを空にし、Promise なら完了値を取り出す
    pub(crate) finish_at_boundary: bool,
    // 引数を含む局所変数の数。引数は 0 から順に並ぶ
    pub(crate) locals: u32,
    pub(crate) body: Vec<Stmt>,
}

/// 値の中身に対する演算。結果の型の番号は演算ごとに決まる
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prim {
    // 型の番号と中身（int）、型の番号と中身から作る値
    Tag,
    Payload,
    MakeV,
    // int。範囲外は __overflow、0 による除算は __div_zero を呼ぶ
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    // bool
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Not,
    // int。ビット演算（右は論理シフト）
    And,
    Or,
    Shl,
    Shr,
    // メモリ。番地は中身。Load は int、Store は unit
    Load,
    Store,
    Load8,
    Store8,
    // 新しいメモリ（引数のバイト数以上）の先頭の番地
    Pages,
    // 確保の状態を置く 16 バイトの番地
    Static,
    // (fd, 番地, 長さ) を書く
    Write,
    // 文字列を実行時エラーとして出し、終わる
    Fail,
    // 1970 年からのミリ秒
    Now,
    Read,
    FFromInt, FToInt, FAdd, FSub, FMul, FDiv, FNeg, FFloor, FSqrt,
    FEq, FLt, FLe,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Const(i64, i64),
    Str(String),
    Local(u32),
    // 関数を間接に呼ぶための番号（int）。関数の値の先頭に入れる
    Func(String),
    Prim(Prim, Vec<Expr>),
    Call(String, Vec<Expr>),
    // 関数の値を呼ぶ。関数の値そのものを先頭の引数に渡す
    CallIndirect(Box<Expr>, Vec<Expr>),
    // 条件は中身が 0 でなければ真
    If(Box<Expr>, Box<Block>, Box<Block>),
    Block(Box<Block>),
    // 塊を評価する。中の Escape で抜けると unit になる。handle の本体に使う
    Catch(Box<Block>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub(crate) stmts: Vec<Stmt>,
    pub(crate) value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Set(u32, Expr),
    Eval(Expr),
    While(Expr, Vec<Stmt>),
    Break,
    Continue,
    Return(Expr),
    // いちばん内側の Catch から unit で抜ける
    Escape,
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Int => "int",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Str => "String",
            Self::Unit => "unit",
            Self::Value => "value",
            Self::Other => "other",
        })
    }
}

impl Program {
    pub fn imports(&self) -> &[Import] {
        &self.imports
    }
    pub fn functions(&self) -> &[Function] {
        &self.functions
    }
}

impl Import {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn param_types(&self) -> &[ValueType] {
        &self.param_types
    }
    pub fn result_type(&self) -> ValueType {
        self.result_type
    }
}

fn boundary(t: ValueType) -> bool {
    matches!(t, ValueType::Int | ValueType::Number | ValueType::Bool | ValueType::Str | ValueType::Unit | ValueType::Value)
}

// 型検査は number の演算の引数を #num で包む。リテラルはそのまま残る。
fn numeric(e: &Ast) -> bool {
    matches!(e.peel(), Ast::Num(_)) || matches!(e.peel(), Ast::Call(n, _) if n == "#num")
}

impl Function {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn param_types(&self) -> &[ValueType] {
        &self.param_types
    }
    pub fn result_type(&self) -> ValueType {
        self.result_type
    }
    pub fn body(&self) -> &[Stmt] {
        &self.body
    }
    /// 利用者が書いた関数で、引数と戻り値を WASM の境界で渡せるか
    pub fn boundary(&self) -> bool {
        !self.name.starts_with("__")
            && self.param_types.len() == self.params as usize
            && self
                .param_types
                .iter()
                .all(|t| boundary(*t) && *t != ValueType::Unit)
            && boundary(self.result_type)
    }
    // 本体に現れるすべての文と式（入れ子を含む）を、出現順に f に渡す
    pub fn each<'a>(&'a self, f: &mut impl FnMut(Node<'a>)) {
        each_stmts(&self.body, f)
    }
    /// 呼ぶ関数の名前。算術の範囲外などで呼ぶランタイムの関数も含む
    pub fn calls(&self) -> Vec<&str> {
        let mut names = Vec::new();
        if self.finish_at_boundary { names.push("__finish_entry"); }
        if self.result_type == ValueType::Int || self.param_types.contains(&ValueType::Int) { names.push("__int_in"); }
        if self.result_type == ValueType::Number || self.param_types.contains(&ValueType::Number) { names.push("__num_checked"); }
        self.each(&mut |n| match n {
            Node::Expr(Expr::Call(n, _) | Expr::Func(n)) => names.push(n.as_str()),
            Node::Expr(Expr::Prim(Prim::Add | Prim::Sub | Prim::Mul, _)) => {
                names.push("__overflow")
            }
            Node::Expr(Expr::Prim(Prim::Div | Prim::Rem, _)) => {
                names.push("__overflow");
                names.push("__div_zero")
            }
            _ => {}
        });
        names
    }
}

#[derive(Clone, Copy)]
pub enum Node<'a> {
    Stmt(&'a Stmt),
    Expr(&'a Expr),
}

fn each_stmts<'a>(stmts: &'a [Stmt], f: &mut impl FnMut(Node<'a>)) {
    for s in stmts {
        f(Node::Stmt(s));
        match s {
            Stmt::Set(_, e) | Stmt::Eval(e) | Stmt::Return(e) => each_expr(e, f),
            Stmt::Break | Stmt::Continue | Stmt::Escape => {}
            Stmt::While(c, body) => {
                each_expr(c, f);
                each_stmts(body, f);
            }
        }
    }
}

fn each_expr<'a>(e: &'a Expr, f: &mut impl FnMut(Node<'a>)) {
    f(Node::Expr(e));
    match e {
        Expr::Const(..) | Expr::Str(_) | Expr::Local(_) | Expr::Func(_) => {}
        Expr::Prim(_, args) | Expr::Call(_, args) => args.iter().for_each(|a| each_expr(a, f)),
        Expr::CallIndirect(g, args) => {
            each_expr(g, f);
            args.iter().for_each(|a| each_expr(a, f));
        }
        Expr::If(c, t, e) => {
            each_expr(c, f);
            each_block(t, f);
            each_block(e, f);
        }
        Expr::Block(b) | Expr::Catch(b) => each_block(b, f),
    }
}

fn each_block<'a>(b: &'a Block, f: &mut impl FnMut(Node<'a>)) {
    each_stmts(&b.stmts, f);
    each_expr(&b.value, f);
}

fn value_type(e: &Ast) -> ValueType {
    match e.peel() {
        Ast::Var(n) if n == "int" => ValueType::Int,
        Ast::Var(n) if n == "number" => ValueType::Number,
        Ast::Var(n) if n == "bool" => ValueType::Bool,
        Ast::Var(n) if n == "String" => ValueType::Str,
        Ast::Var(n) if n == "unit" => ValueType::Unit,
        Ast::Refine(base, _, _) => value_type(base),
        _ => ValueType::Other,
    }
}

fn ret_type(f: &parser::Fn) -> ValueType {
    f.ret.as_ref().map_or(ValueType::Unit, value_type)
}

// IR を組み立てる短い名前
fn int(n: i64) -> Expr {
    Expr::Const(INT, n)
}
fn prim(p: Prim, args: Vec<Expr>) -> Expr {
    Expr::Prim(p, args)
}
fn call(n: &str, args: Vec<Expr>) -> Expr {
    Expr::Call(n.to_string(), args)
}
fn add(a: Expr, b: Expr) -> Expr {
    prim(Prim::Add, vec![a, b])
}
fn mkv(tag: i64, p: Expr) -> Expr {
    prim(Prim::MakeV, vec![int(tag), p])
}
fn block(stmts: Vec<Stmt>, value: Expr) -> Expr {
    Expr::Block(Box::new(Block { stmts, value }))
}
fn leaf(value: Expr) -> Box<Block> {
    Box::new(Block {
        stmts: vec![],
        value,
    })
}
fn unit() -> Expr {
    Expr::Const(UNIT, 0)
}
// 番地の値を読む・書く。番地は何度も読むので、局所変数か定数にしておく
fn getv(addr: Expr) -> Expr {
    prim(
        Prim::MakeV,
        vec![
            prim(Prim::Load, vec![addr.clone()]),
            prim(Prim::Load, vec![add(addr, int(8))]),
        ],
    )
}
fn setv(addr: Expr, v: Expr) -> Vec<Stmt> {
    vec![
        Stmt::Eval(prim(
            Prim::Store,
            vec![addr.clone(), prim(Prim::Tag, vec![v.clone()])],
        )),
        Stmt::Eval(prim(
            Prim::Store,
            vec![add(addr, int(8)), prim(Prim::Payload, vec![v])],
        )),
    ]
}

// 型検査した関数（利用者と std）とランタイムの関数
struct Source<'a> {
    f: &'a parser::Fn,
    runtime: bool,
}

struct Ctx<'a> {
    sources: HashMap<&'a str, Source<'a>>,
    // 型の名前から宣言。境界の型（BoundaryType）を作るのに使う
    datas: HashMap<&'a str, &'a parser::Data>,
    // 構築子の名前から (番号, フィールド名)
    ctors: HashMap<&'a str, (i64, Vec<&'a str>)>,
    ctor_list: Vec<(&'a Name, Vec<&'a str>)>,
    types: HashSet<&'a str>,
    // 変換の途中で作った関数（無名関数）
    made: Vec<Function>,
    lambdas: usize,
    // 型検査が記録した、JavaScript で待つ呼び出しと async にする本体（js.rs と同じ所で待つ）
    info: &'a crate::typeck::Info,
    // コルーチンに panic を伝えるため、要求のない関数の呼び出しも検査する
    panic_checks: bool,
}

pub fn lower(p: &parser::Program) -> R<Program> {
    lower_selected(p, None, &crate::typeck::Info::default())
}

/// exports から辿れる関数だけを変換する。None はすべての関数を変換する。
pub fn lower_selected(p: &parser::Program, exports: Option<&[String]>, info: &crate::typeck::Info) -> R<Program> {
    let runtime = parser::parse(RUNTIME)?;
    let mut sources = HashMap::new();
    for f in &runtime.fns {
        sources.insert(f.name.as_str(), Source { f, runtime: true });
    }
    for f in &p.fns {
        sources.insert(f.name.as_str(), Source { f, runtime: false });
    }
    let ctor_list: Vec<(&Name, Vec<&str>)> = p
        .datas
        .iter()
        .flat_map(|d| &d.ctors)
        .map(|c| {
            (
                &c.name,
                c.fields.iter().map(|f| f.0.as_str()).collect(),
            )
        })
        .collect();
    let mut cx = Ctx {
        ctors: ctor_list
            .iter()
            .enumerate()
            .map(|(i, (n, fs))| (n.as_str(), (i as i64, fs.clone())))
            .collect(),
        ctor_list,
        types: p.datas.iter().map(|d| d.name.as_str()).collect(),
        datas: p.datas.iter().map(|d| (d.name.as_str(), d)).collect(),
        sources,
        made: Vec::new(),
        lambdas: 0,
        info,
        panic_checks: !info.awaits.is_empty() || p.fns.iter().any(|f| has_async(&f.effects)),
    };
    if exports.is_none() && (!p.datas.is_empty() || !p.tests.is_empty()) {
        return Err("IR は型宣言とテスト宣言に未対応".into());
    }
    let mut pending: Vec<String> = match exports {
        Some(names) => {
            let mut seen = HashSet::new();
            for name in names {
                if !p.fns.iter().any(|f| f.name == *name) {
                    return Err(format!("公開する関数 {name} が無い").into());
                }
                if matches!(
                    name.as_str(),
                    "memory" | "__hibana_alloc" | "__hibana_reset"
                ) {
                    return Err(format!("公開名 {name} は WASM ABI の予約名").into());
                }
                if !seen.insert(name) {
                    return Err(format!("公開する関数 {name} が重複している").into());
                }
            }
            names.to_vec()
        }
        None => p.fns.iter().map(|f| f.name.to_string()).collect(),
    };
    let exported = |name: &str| exports.is_some_and(|names| names.iter().any(|n| n == name));
    let mut visited = HashSet::new();
    let mut functions = Vec::new();
    let mut imports = Vec::new();
    let mut descs = HashMap::new();
    loop {
        while let Some(name) = pending.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            let js = cx
                .sources
                .get(name.as_str())
                .map(|s| s.f)
                .filter(|f| f.js.is_some());
            let mut function = match js {
                Some(f) => {
                    let (param_types, result_type) = cx.boundary(f, &mut descs);
                    if param_types.iter().chain([&result_type]).any(|t| *t == ValueType::Value) {
                        pending.extend(["__val_in".to_string(), "__val_out".to_string()]);
                    }
                    if param_types.contains(&ValueType::Int) || result_type == ValueType::Int { pending.push("__int_in".into()); }
                    if param_types.contains(&ValueType::Number) || result_type == ValueType::Number {
                        pending.push("__num_checked".into());
                    }
                    if exported(&name) {
                        return Err("外部関数を直接公開できない".into());
                    }
                    // WASM の境界は int・number・String・bool・unit と、それらの Vec と構造体（BoundaryType）
                    if !param_types
                        .iter()
                        .chain([&result_type])
                        .all(|t| boundary(*t))
                    {
                        return Err(format!("IR は関数 {} の境界の型に未対応", f.name).into());
                    }
                    if param_types.iter().chain([&result_type]).any(|t| matches!(t, ValueType::Str)) {
                        pending.extend(["__str_in".to_string(), "__str_out".to_string()]);
                    }
                    imports.push(Import {
                        name: f.name.to_string(),
                        param_types,
                        result_type,
                    });
                    continue;
                }
                None if cx.sources.contains_key(name.as_str()) => cx.function(&name)?,
                None => cx.generated(&name)?,
            };
            if exported(&name) {
                if let Some(f) = cx.sources.get(name.as_str()).map(|s| s.f)
                    && f.implicit == 0
                {
                    let (param_types, result_type) = cx.boundary(f, &mut descs);
                    if param_types.iter().chain([&result_type]).any(|t| *t == ValueType::Value) {
                        pending.extend(["__val_in".to_string(), "__val_out".to_string()]);
                    }
                    function.param_types = param_types;
                    function.result_type = result_type;
                }
                if !function.boundary() {
                    return Err(format!("IR は関数 {} の境界の型に未対応", function.name).into());
                }
                if function.param_types.iter().chain([&function.result_type]).any(|t| matches!(t, ValueType::Str))
                {
                    pending.extend(["__str_in".to_string(), "__str_out".to_string()]);
                }
            }
            pending.extend(function.calls().into_iter().map(String::from));
            for f in std::mem::take(&mut cx.made) {
                pending.extend(f.calls().into_iter().map(String::from));
                visited.insert(f.name.clone());
                functions.push(f);
            }
            functions.push(function);
        }
        // 同期の main でも spawn した仕事が残る。main の入口でその仕事を動かす
        let asyncs = ["__co_new", "__spawn", "__all", "__aperform"].iter().any(|n| visited.contains(*n));
        let Some(main) = functions.iter_mut().find(|f| f.name == "main" && !f.finish_at_boundary) else { break };
        if !asyncs { break; }
        main.finish_at_boundary = true;
        pending.push("__finish_entry".into());
    }
    // 宣言の順（ランタイム、利用者の関数）に並べ、出力を決まった形にする
    let order: HashMap<&str, usize> = runtime
        .fns
        .iter()
        .chain(&p.fns)
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), i))
        .collect();
    functions.sort_by(|a, b| {
        let key = |f: &Function| {
            (
                order.get(f.name.as_str()).copied().unwrap_or(usize::MAX),
                f.name.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    imports.sort_by_key(|f| order[f.name.as_str()]);
    Ok(Program { functions, imports, boundary: descs })
}

impl<'a> Ctx<'a> {
    fn function(&mut self, name: &str) -> R<Function> {
        let src = &self.sources[name];
        self.lower_fn(src.f, src.runtime, name, false)
    }

    // 関数 f を name として変換する。variant は要求の多相な関数の async 版（js.rs の f$a）。
    // async 関数（Async を要求する関数と async 版）はコルーチンにし、入口の関数を返す（Ctx::coroutine）
    fn lower_fn(&mut self, f: &'a parser::Fn, runtime: bool, name: &str, variant: bool) -> R<Function> {
        if f.op.is_some() {
            return Err(format!("IR は要求の操作 {} に未対応", f.name).into());
        }
        let mut l = Lower {
            cx: self,
            vars: HashMap::new(),
            locals: 0,
            raw: runtime,
            handles: Vec::new(),
            loop_handle_depth: None,
            muts: HashSet::new(),
            variant,
        };
        for (n, _) in &f.params {
            let i = l.local();
            l.vars.insert(n.clone(), i);
        }
        let b = l.block(&f.body)?;
        let locals = l.locals;
        let mut body = b.stmts;
        body.push(Stmt::Return(b.value));
        let mut function = Function {
            name: name.to_string(),
            params: f.params.len() as u32,
            param_types: f.params.iter().map(|(_, t)| value_type(t)).collect(),
            result_type: ret_type(f),
            finish_at_boundary: !runtime && !variant && has_async(&f.effects),
            locals,
            body,
        };
        if !runtime && (variant || has_async(&f.effects)) {
            return Ok(self.coroutine(function));
        }
        if !runtime {
            self.shadow(&mut function);
        }
        Ok(function)
    }

    // 変換の中で名前だけを決めて呼ぶ関数を作る
    fn generated(&mut self, name: &str) -> R<Function> {
        let other = |params: u32| Function {
            name: name.to_string(),
            params,
            param_types: vec![],
            result_type: ValueType::Other,
            finish_at_boundary: false,
            locals: params,
            body: vec![],
        };
        let eq = |a: Expr, n: i64| prim(Prim::Eq, vec![a, int(n)]);
        // 構築子の番号から名前、(番号, 位置) からフィールド名
        if matches!(name, "__ctor_name" | "__ctor_field" | "__ctor_kind") {
            let mut f = other(if name == "__ctor_field" { 2 } else { 1 });
            for (i, (cn, fields)) in self.ctor_list.iter().enumerate() {
                if name == "__ctor_kind" {
                    let kind = match cn.structure_style() { Some(crate::frontend::name::StructureStyle::Record) => 1, Some(crate::frontend::name::StructureStyle::Tuple) => 2, None => 0 };
                    f.body.push(if_return(eq(Expr::Local(0), i as i64), int(kind)));
                    continue;
                }
                if name == "__ctor_name" {
                    f.body.push(if_return(
                        eq(Expr::Local(0), i as i64),
                        Expr::Str(cn.structure_owner().unwrap_or_else(|| (*cn).clone()).to_string()),
                    ));
                    continue;
                }
                for (k, field) in fields.iter().enumerate() {
                    let cond = Expr::If(
                        Box::new(eq(Expr::Local(0), i as i64)),
                        leaf(eq(Expr::Local(1), k as i64)),
                        leaf(Expr::Const(BOOL, 0)),
                    );
                    f.body.push(if_return(cond, Expr::Str(field.to_string())));
                }
            }
            f.body.push(Stmt::Return(if name == "__ctor_kind" { int(0) } else { Expr::Str(String::new()) }));
            return Ok(f);
        }
        // 名前でフィールドを読む。構築子ごとに位置が違うので番号で振り分ける
        if let Some(field) = name.strip_prefix("__field_") {
            let mut f = other(1);
            f.locals = 2;
            f.body
                .push(Stmt::Set(1, prim(Prim::Load, vec![Expr::Local(0)])));
            for (i, (_, fields)) in self.ctor_list.iter().enumerate() {
                if let Some(k) = fields.iter().position(|x| *x == field) {
                    let at = add(Expr::Local(0), int(16 + 16 * k as i64));
                    f.body
                        .push(if_return(eq(Expr::Local(1), i as i64), getv(at)));
                }
            }
            f.body.push(Stmt::Return(call(
                "__die",
                vec![Expr::Str(format!("フィールド {field} が無い"))],
            )));
            return Ok(f);
        }
        // 要求の多相な関数の async 版（js.rs の f$a）。要求の変数を通した呼び出しを待つコルーチンにする
        if let Some(base) = name.strip_suffix("$a")
            && let Some(src) = self.sources.get(base).filter(|s| !s.runtime)
        {
            let f = src.f;
            return self.lower_fn(f, false, name, true);
        }
        // 名前の付いた関数を値として使う時の入口
        if let Some(target) = name.strip_prefix("__w_") {
            let g = self
                .sources
                .get(target)
                .ok_or_else(|| format!("IR に関数 {target} が無い"))?
                .f;
            let n = (g.params.len() - g.implicit) as u32;
            let mut f = other(1 + n);
            let mut args: Vec<Expr> = (0..g.implicit).map(|_| unit()).collect();
            args.extend((1..=n).map(Expr::Local));
            f.body.push(Stmt::Return(call(target, args)));
            return Ok(f);
        }
        Err(format!("IR に関数 {name} が無い").into())
    }

    // 境界の関数の引数と戻り値の型。Vec と構造体は Value にし、形を descs に記録する
    fn boundary(&self, f: &parser::Fn, descs: &mut HashMap<String, (Vec<BoundaryType>, BoundaryType)>) -> (Vec<ValueType>, ValueType) {
        let ty = |t: Option<&Ast>| match t.map_or(Some(BoundaryType::Unit), |t| self.desc(t, &mut Vec::new())) {
            Some(d) => (d.value_type(), Some(d)),
            None => (t.map_or(ValueType::Unit, value_type), None),
        };
        let params: Vec<_> = f.params.iter().map(|(_, t)| ty(Some(t))).collect();
        let result = ty(f.ret.as_ref());
        if let (Some(ps), Some(r)) = (params.iter().map(|p| p.1.clone()).collect::<Option<Vec<_>>>(), result.1.clone()) {
            descs.insert(f.name.to_string(), (ps, r));
        }
        (params.into_iter().map(|p| p.0).collect(), result.0)
    }

    // 境界で渡せる型の形。int・number・bool・String・unit と、それらの Vec と、型引数の無い構造体。
    // 自分を含む構造体（seen にある型）や、Ref・関数の型を含むものは None
    fn desc(&self, t: &Ast, seen: &mut Vec<Name>) -> Option<BoundaryType> {
        Some(match t.peel() {
            Ast::Var(n) if n == "int" => BoundaryType::Int,
            Ast::Var(n) if n == "number" => BoundaryType::Number,
            Ast::Var(n) if n == "bool" => BoundaryType::Bool,
            Ast::Var(n) if n == "String" => BoundaryType::Str,
            Ast::Var(n) if n == "unit" => BoundaryType::Unit,
            Ast::Refine(base, _, _) => self.desc(base, seen)?,
            Ast::Call(n, xs) if n == "Vec" && xs.len() == 1 => BoundaryType::Vec(Box::new(self.desc(&xs[0], seen)?)),
            Ast::Var(n) => {
                let d = self.datas.get(n.as_str())?;
                let [c] = d.ctors.as_slice() else { return None };
                if !d.params.is_empty() || c.name.structure_owner().is_none() || seen.contains(n) {
                    return None;
                }
                let (k, _) = self.ctors.get(c.name.as_str())?;
                seen.push(n.clone());
                let fields = c.fields.iter().map(|(f, t)| Some((f.to_string(), self.desc(t, seen)?))).collect::<Option<Vec<_>>>();
                seen.pop();
                BoundaryType::Struct(*k, fields?)
            }
            _ => return None,
        })
    }

    // GC が動かない呼び出し先。ランタイムの関数のうち利用者の関数を呼ばないものと、変換の中で作る関数
    fn quiet(&self, n: &str) -> bool {
        let runtime = self.sources.get(n).is_some_and(|s| s.runtime);
        (runtime && !CALLBACKS.contains(&n)) || n.starts_with("__field_") || n.starts_with("__ctor_")
    }

    // 利用者の関数（ランタイム以外）に GC のシャドースタックを付ける（runtime.hb の先頭の説明）
    fn shadow(&self, f: &mut Function) {
        let mut s = Shadow { cx: self, next: f.locals, fp: 0, ret: 0, size: 0 };
        let body = s.spill_stmts(std::mem::take(&mut f.body));
        s.size = s.next;
        s.fp = s.next;
        s.ret = s.next + 1;
        let mut out = vec![Stmt::Set(s.fp, call("__frame_push", vec![int(s.size as i64)]))];
        for i in 0..f.params {
            out.extend(setv(s.slot(i), Expr::Local(i)));
        }
        out.push(poll());
        out.extend(s.frame_stmts(body));
        f.body = out;
        f.locals = s.next + 2;
    }
}

// 利用者の関数を呼ぶランタイムの関数。中で GC が動きうる
const CALLBACKS: &[&str] = &["__all", "__perform", "__spawn", "__task_then", "__co_step"];

// 前回の GC の後に確保したバイト数が上限を超えていれば GC を動かす。安全な位置（関数の入口とループの先頭）に置く
fn poll() -> Stmt {
    let st = |k: i64| prim(Prim::Load, vec![add(prim(Prim::Static, vec![]), int(k))]);
    Stmt::Eval(Expr::If(
        Box::new(prim(Prim::Gt, vec![st(96), st(104)])),
        Box::new(Block { stmts: vec![Stmt::Eval(call("__gc", vec![]))], value: unit() }),
        leaf(unit()),
    ))
}

// シャドースタックを付ける変換。局所変数 0..size が区画に写す変数で、fp は区画の位置、ret は戻り値を置く変数
struct Shadow<'c, 'a> {
    cx: &'c Ctx<'a>,
    next: u32,
    fp: u32,
    ret: u32,
    size: u32,
}

impl Shadow<'_, '_> {
    fn slot(&self, i: u32) -> Expr {
        add(Expr::Local(self.fp), int(16 * i as i64))
    }

    fn may_gc(&self, e: &Expr) -> bool {
        match e {
            Expr::Call(n, args) => !self.cx.quiet(n) || args.iter().any(|a| self.may_gc(a)),
            Expr::CallIndirect(..) => true,
            Expr::Prim(_, args) => args.iter().any(|a| self.may_gc(a)),
            Expr::If(c, t, e) => self.may_gc(c) || self.block_may_gc(t) || self.block_may_gc(e),
            Expr::Block(b) | Expr::Catch(b) => self.block_may_gc(b),
            Expr::Const(..) | Expr::Str(_) | Expr::Local(_) | Expr::Func(_) => false,
        }
    }

    fn block_may_gc(&self, b: &Block) -> bool {
        self.may_gc(&b.value)
            || b.stmts.iter().any(|s| match s {
                Stmt::Set(_, e) | Stmt::Eval(e) | Stmt::Return(e) => self.may_gc(e),
                Stmt::While(..) => true,
                Stmt::Break | Stmt::Continue | Stmt::Escape => false,
            })
    }

    // 引数の評価の途中で GC が動くと、評価済みの引数は局所変数に無いので根にならない。
    // 後の引数が GC を起こしうるなら、前の引数を局所変数に移す（評価の順は変えない）
    fn spill_args(&mut self, mut args: Vec<Expr>) -> (Vec<Stmt>, Vec<Expr>) {
        let mut pre = Vec::new();
        for i in 0..args.len() {
            let constant = matches!(args[i], Expr::Const(..) | Expr::Str(_) | Expr::Func(_));
            if !constant && args[i + 1..].iter().any(|a| self.may_gc(a)) {
                let t = self.next;
                self.next += 1;
                pre.push(Stmt::Set(t, std::mem::replace(&mut args[i], Expr::Local(t))));
            }
        }
        (pre, args)
    }

    fn spill(&mut self, e: Expr) -> Expr {
        let wrap = |pre: Vec<Stmt>, e: Expr| if pre.is_empty() { e } else { block(pre, e) };
        match e {
            Expr::Call(n, args) => {
                let args = args.into_iter().map(|a| self.spill(a)).collect();
                let (pre, args) = self.spill_args(args);
                wrap(pre, Expr::Call(n, args))
            }
            Expr::Prim(p, args) => {
                let args = args.into_iter().map(|a| self.spill(a)).collect();
                let (pre, args) = self.spill_args(args);
                wrap(pre, Expr::Prim(p, args))
            }
            Expr::CallIndirect(f, args) => {
                let mut all = vec![self.spill(*f)];
                all.extend(args.into_iter().map(|a| self.spill(a)));
                let (pre, mut all) = self.spill_args(all);
                let f = all.remove(0);
                wrap(pre, Expr::CallIndirect(Box::new(f), all))
            }
            Expr::If(c, t, e) => Expr::If(Box::new(self.spill(*c)), Box::new(self.spill_block(*t)), Box::new(self.spill_block(*e))),
            Expr::Block(b) => Expr::Block(Box::new(self.spill_block(*b))),
            Expr::Catch(b) => Expr::Catch(Box::new(self.spill_block(*b))),
            e => e,
        }
    }

    fn spill_block(&mut self, b: Block) -> Block {
        Block { stmts: self.spill_stmts(b.stmts), value: self.spill(b.value) }
    }

    fn spill_stmts(&mut self, ss: Vec<Stmt>) -> Vec<Stmt> {
        ss.into_iter()
            .map(|s| match s {
                Stmt::Set(i, e) => Stmt::Set(i, self.spill(e)),
                Stmt::Eval(e) => Stmt::Eval(self.spill(e)),
                Stmt::Return(e) => Stmt::Return(self.spill(e)),
                Stmt::While(c, body) => Stmt::While(self.spill(c), self.spill_stmts(body)),
                s => s,
            })
            .collect()
    }

    // 局所変数への代入を区画にも書き、ループの先頭で GC を確かめ、戻る前に区画を外す
    fn frame(&self, e: Expr) -> Expr {
        match e {
            Expr::Call(n, args) => Expr::Call(n, args.into_iter().map(|a| self.frame(a)).collect()),
            Expr::Prim(p, args) => Expr::Prim(p, args.into_iter().map(|a| self.frame(a)).collect()),
            Expr::CallIndirect(f, args) => Expr::CallIndirect(Box::new(self.frame(*f)), args.into_iter().map(|a| self.frame(a)).collect()),
            Expr::If(c, t, e) => Expr::If(Box::new(self.frame(*c)), Box::new(self.frame_block(*t)), Box::new(self.frame_block(*e))),
            Expr::Block(b) => Expr::Block(Box::new(self.frame_block(*b))),
            Expr::Catch(b) => Expr::Catch(Box::new(self.frame_block(*b))),
            e => e,
        }
    }

    fn frame_block(&self, b: Block) -> Block {
        Block { stmts: self.frame_stmts(b.stmts), value: self.frame(b.value) }
    }

    fn frame_stmts(&self, ss: Vec<Stmt>) -> Vec<Stmt> {
        let mut out = Vec::new();
        for s in ss {
            match s {
                Stmt::Set(i, e) => {
                    out.push(Stmt::Set(i, self.frame(e)));
                    if i < self.size {
                        out.extend(setv(self.slot(i), Expr::Local(i)));
                    }
                }
                Stmt::Eval(e) => out.push(Stmt::Eval(self.frame(e))),
                Stmt::While(c, body) => {
                    let mut stmts = vec![poll()];
                    stmts.extend(self.frame_stmts(body));
                    out.push(Stmt::While(self.frame(c), stmts));
                }
                Stmt::Return(e) => {
                    out.push(Stmt::Set(self.ret, self.frame(e)));
                    out.push(Stmt::Eval(prim(Prim::Store, vec![add(prim(Prim::Static, vec![]), int(64)), Expr::Local(self.fp)])));
                    out.push(Stmt::Return(Expr::Local(self.ret)));
                }
                s => out.push(s),
            }
        }
        out
    }
}

// 式に付いた識別子（いちばん内側の Expr::Id）。型検査は待つ呼び出しをこの識別子で記録する
fn node_id(mut e: &Ast) -> Option<parser::NodeId> {
    let mut id = None;
    loop {
        match e {
            Ast::At(_, x) | Ast::InModule(_, x) => e = x,
            Ast::Id(i, x) => {
                id = Some(*i);
                e = x;
            }
            _ => return id,
        }
    }
}

// Async を要求する関数か（js.rs の is_async）
fn has_async(effs: &[Ast]) -> bool {
    effs.iter().any(|e| matches!(e.peel(), Ast::Var(n) if n == "Async"))
}

// 要求の変数（<E: Effect>）を持つ関数（js.rs の is_poly）
fn is_poly(f: &parser::Fn) -> bool {
    f.params[..f.implicit].iter().any(|(_, t)| matches!(t.peel(), Ast::Var(k) if k == "Effect"))
}

fn if_return(cond: Expr, v: Expr) -> Stmt {
    Stmt::Eval(Expr::If(
        Box::new(cond),
        Box::new(Block {
            stmts: vec![Stmt::Return(v)],
            value: unit(),
        }),
        leaf(unit()),
    ))
}

struct Lower<'c, 'a> {
    cx: &'c mut Ctx<'a>,
    vars: HashMap<Name, u32>,
    locals: u32,
    // ランタイムの関数。演算子は中身の演算になる
    raw: bool,
    // 囲んでいる with の番号を持つ局所変数と、ループ本体に入る時のその数
    handles: Vec<u32>,
    loop_handle_depth: Option<usize>,
    // mut で宣言した変数。値を読んで他へ渡す時は __share で Vec の所有の印を消す
    muts: HashSet<Name>,
    // 要求の多相な関数の async 版（js.rs の variant）を変換している
    variant: bool,
}

// 値を捨てる。unit の塊は中の文を並べ直す
fn eval(out: &mut Vec<Stmt>, e: Expr) {
    match e {
        Expr::Const(..) => {}
        Expr::Block(b) if matches!(b.value, Expr::Const(..)) => out.extend(b.stmts),
        e => out.push(Stmt::Eval(e)),
    }
}

impl Lower<'_, '_> {
    // 中断しうる呼び出し。戻った後に中断中なら、handle の本体を抜けるか関数から戻る
    fn checked(&mut self, c: Expr) -> Expr {
        if self.raw {
            return c;
        }
        let t = self.local();
        let exit = if !self.handles.is_empty() {
            Stmt::Escape
        } else {
            Stmt::Return(unit())
        };
        let aborting = prim(Prim::Load, vec![add(prim(Prim::Static, vec![]), int(24))]);
        let check = Expr::If(
            Box::new(aborting),
            Box::new(Block {
                stmts: vec![exit],
                value: unit(),
            }),
            leaf(unit()),
        );
        block(vec![Stmt::Set(t, c), Stmt::Eval(check)], Expr::Local(t))
    }

    fn local(&mut self) -> u32 {
        self.locals += 1;
        self.locals - 1
    }

    // 値を一時変数に置き、何度でも読めるようにする
    fn temp(&mut self, e: Expr, out: &mut Vec<Stmt>) -> Expr {
        if let Expr::Local(_) | Expr::Const(..) = e {
            return e;
        }
        let t = self.local();
        out.push(Stmt::Set(t, e));
        Expr::Local(t)
    }

    fn block(&mut self, b: &parser::Block) -> R<Block> {
        let saved = self.vars.clone();
        let mut stmts = Vec::new();
        for s in &b.stmts {
            match s {
                AstStmt::Let { name, init, mutable, .. } => {
                    let e = self.expr(init)?;
                    if *mutable && !self.raw {
                        self.muts.insert(name.clone());
                    }
                    let i = self.local();
                    stmts.push(Stmt::Set(i, e));
                    self.vars.insert(name.clone(), i);
                }
                AstStmt::Expr(e) => self.effect(e, &mut stmts)?,
            }
        }
        let value = match &b.tail {
            Some(e) => self.expr(e)?,
            None => unit(),
        };
        self.vars = saved;
        Ok(Block { stmts, value })
    }

    fn loop_body(&mut self, b: &parser::Block) -> R<Vec<Stmt>> {
        let saved = self.loop_handle_depth.replace(self.handles.len());
        let b = self.block(b);
        self.loop_handle_depth = saved;
        let b = b?;
        let mut stmts = b.stmts;
        eval(&mut stmts, b.value);
        Ok(stmts)
    }

    // 文の位置の式。値は捨てる
    fn effect(&mut self, e: &Ast, out: &mut Vec<Stmt>) -> R<()> {
        match e.peel() {
            Ast::Assign(n, v) => {
                let i = *self
                    .vars
                    .get(n)
                    .ok_or_else(|| format!("IR は変数 {n} に未対応"))?;
                // x = push(x, e) と x = set_at(x, i, e) は、x が所有する Vec ならその場で書き換える（js.rs の $opush と同じ）
                let own = self.muts.contains(n);
                let v = match (v.self_push(n), v.self_set(n)) {
                    (Some(e), _) if own && !self.cx.sources.contains_key("push") => {
                        call("__push_own", vec![Expr::Local(i), self.expr(e)?])
                    }
                    (_, Some((k, e))) if own && !self.cx.sources.contains_key("set_at") => {
                        let k = self.expr(k)?;
                        call("__set_at_own", vec![Expr::Local(i), k, self.expr(e)?])
                    }
                    _ => self.expr(v)?,
                };
                out.push(Stmt::Set(i, v));
            }
            Ast::While(c, body) => {
                let c = self.expr(c)?;
                let body = self.loop_body(body)?;
                out.push(Stmt::While(c, body));
            }
            Ast::For(x, from, to, body) => {
                let saved = self.vars.clone();
                let i = self.local();
                let end = self.local();
                let x_local = self.local();
                let (start, limit, item) = match to {
                    Some(to) => {
                        let from = self.expr(from)?;
                        let to = self.expr(to)?;
                        (from, to, Expr::Local(i))
                    }
                    None => {
                        let v = self.expr(from)?;
                        let v = self.temp(v, out);
                        let at = add(
                            add(v.clone(), int(8)),
                            prim(Prim::Mul, vec![Expr::Local(i), int(16)]),
                        );
                        (
                            int(0),
                            prim(Prim::Load, vec![v]),
                            getv(at),
                        )
                    }
                };
                out.push(Stmt::Set(i, start));
                out.push(Stmt::Set(end, limit));
                // 今の要素を束縛した後で添字を進める。continue は条件の評価に戻る
                let mut stmts = vec![
                    Stmt::Set(x_local, item),
                    Stmt::Set(i, add(Expr::Local(i), int(1))),
                ];
                self.vars.insert(x.clone(), x_local);
                stmts.extend(self.loop_body(body)?);
                self.vars = saved;
                let c = prim(Prim::Lt, vec![Expr::Local(i), Expr::Local(end)]);
                out.push(Stmt::While(c, stmts));
            }
            Ast::Break | Ast::Continue => {
                let depth = self.loop_handle_depth.expect("ループの本体の中だけ");
                // 行き先との間にある with を内側から外す
                for &h in self.handles[depth..].iter().rev() {
                    out.push(Stmt::Eval(call("__handle_end", vec![Expr::Local(h), unit()])));
                }
                out.push(if matches!(e.peel(), Ast::Break) { Stmt::Break } else { Stmt::Continue });
            }
            Ast::Return(v) => {
                let v = match v {
                    Some(v) => self.expr(v)?,
                    None => unit(),
                };
                out.push(Stmt::Return(v));
            }
            _ => {
                let v = self.expr(e)?;
                eval(out, v);
            }
        }
        Ok(())
    }

    // JavaScript で待つ呼び出しか、async にする無名関数の本体か（js.rs の Gen::waits）
    fn waits(&self, id: Option<parser::NodeId>) -> bool {
        id.is_some_and(|id| self.cx.info.awaits.contains(&id) || (self.variant && self.cx.info.var_awaits.contains(&id)))
    }

    // await。js.rs の $k($h, await e) と同じく、待つ前の with の並びを覚え、再開した後で戻す。
    // #await はコルーチンの変換（Ctx::coroutine）が中断と再開に書き換える
    fn await_(&mut self, e: Expr) -> Expr {
        let (h, t) = (self.local(), self.local());
        let handlers = add(prim(Prim::Static, vec![]), int(48));
        block(
            vec![
                Stmt::Set(h, mkv(HANDLERS, prim(Prim::Load, vec![handlers.clone()]))),
                Stmt::Set(t, call("#await", vec![e])),
                Stmt::Eval(prim(Prim::Store, vec![handlers, prim(Prim::Payload, vec![Expr::Local(h)])])),
            ],
            Expr::Local(t),
        )
    }

    // 局所変数 n（番号 i）の値。mut の変数は他へ渡すので、Vec の所有の印を消す
    fn shared(&self, n: &Name, i: u32) -> Expr {
        if !self.muts.contains(n) {
            return Expr::Local(i);
        }
        let vec = prim(Prim::Eq, vec![prim(Prim::Tag, vec![Expr::Local(i)]), int(ARR)]);
        Expr::If(Box::new(vec), leaf(call("__share", vec![Expr::Local(i)])), leaf(Expr::Local(i)))
    }

    // 読むだけで他へ渡さない所（len と添字）。変数なら所有の印を残す
    fn look(&mut self, e: &Ast) -> R<Expr> {
        match e.peel() {
            Ast::Var(n) if self.vars.contains_key(n) => Ok(Expr::Local(self.vars[n])),
            _ => self.expr(e),
        }
    }

    fn args(&mut self, args: &[Ast]) -> R<Vec<Expr>> {
        args.iter().map(|a| self.expr(a)).collect()
    }

    // 値を並べて確保した領域に書き、型の番号を付ける。値を先にすべて局所変数に置いてから確保する。
    // 確保した位置は型の番号の無い整数なので、値の評価の途中で GC が動くと回収されてしまう
    fn object(&mut self, tag: i64, new: Expr, header: i64, items: Vec<Expr>) -> Expr {
        let mut stmts = Vec::new();
        let items: Vec<Expr> = items.into_iter().map(|item| self.temp(item, &mut stmts)).collect();
        let t = self.local();
        stmts.push(Stmt::Set(t, new));
        for (k, item) in items.into_iter().enumerate() {
            stmts.extend(setv(add(Expr::Local(t), int(header + 16 * k as i64)), item));
        }
        block(stmts, mkv(tag, Expr::Local(t)))
    }

    fn expr(&mut self, e: &Ast) -> R<Expr> {
        let id = node_id(e);
        Ok(match e.peel() {
            Ast::Int(n) => int(*n),
            Ast::Num(n) => Expr::Const(NUM, n.to_bits() as i64),
            Ast::Bool(b) => Expr::Const(BOOL, *b as i64),
            Ast::Str(s) => Expr::Str(s.clone()),
            Ast::Var(n) => {
                if let Some(&i) = self.vars.get(n) {
                    return Ok(self.shared(n, i));
                }
                if let Some((i, _)) = self.cx.ctors.get(n.as_str()) {
                    return Ok(mkv(DATA, call("__data_new", vec![int(*i), int(0)])));
                }
                match self.cx.sources.get(n.as_str()) {
                    Some(s) if s.f.alias && s.f.params.is_empty() => call(n, vec![]),
                    Some(_) => mkv(
                        FN,
                        call("__fn_new", vec![Expr::Func(format!("__w_{n}")), int(0)]),
                    ),
                    // 型は名前だけ
                    None => mkv(TY, Expr::Str(n.to_string())),
                }
            }
            Ast::Unary("-", a) if matches!(a.peel(), Ast::Num(_)) => {
                let Ast::Num(n) = a.peel() else { unreachable!() };
                Expr::Const(NUM, (-n).to_bits() as i64)
            }
            Ast::Unary("-", a) if !self.raw && numeric(a) => call("__neg", vec![self.look(a)?]),
            Ast::Unary("-", a) if !self.raw => call("__ineg", vec![self.look(a)?]),
            Ast::Unary("-", a) => prim(Prim::Sub, vec![int(0), self.expr(a)?]),
            Ast::Unary(_, a) => prim(Prim::Not, vec![self.look(a)?]),
            Ast::Binary(op @ ("&&" | "||"), a, b) => {
                let (a, b) = (self.look(a)?, self.look(b)?);
                let (t, f) = if *op == "&&" {
                    (leaf(b), leaf(Expr::Const(BOOL, 0)))
                } else {
                    (leaf(Expr::Const(BOOL, 1)), leaf(b))
                };
                Expr::If(Box::new(a), t, f)
            }
            Ast::Binary(op, a, b) => {
                let number = !self.raw && (numeric(a) || numeric(b));
                // 演算子は値を他へ渡さないので、変数の所有の印を残す
                let (a, b) = (self.look(a)?, self.look(b)?);
                if *op == "**" {
                    return Ok(call(if number { "__value_pow" } else { "__pow" }, vec![a, b]));
                }
                if number && matches!(*op, "-" | "*" | "/" | "%" | "<" | "<=" | ">" | ">=") {
                    let name = match *op { "-" => "__sub", "*" => "__mul", "/" => "__div", "%" => "__rem", "<" => "__lt", "<=" => "__le", ">" => "__gt", _ => "__ge" };
                    return Ok(call(name, vec![a, b]));
                }
                if !self.raw && matches!(*op, "-" | "*") {
                    return Ok(call(if *op == "-" { "__isub" } else { "__imul" }, vec![a, b]));
                }
                let p = match *op {
                    "+" => Prim::Add,
                    "-" => Prim::Sub,
                    "*" => Prim::Mul,
                    "/" => Prim::Div,
                    "%" => Prim::Rem,
                    "==" => Prim::Eq,
                    "!=" => Prim::Ne,
                    "<" => Prim::Lt,
                    "<=" => Prim::Le,
                    ">" => Prim::Gt,
                    ">=" => Prim::Ge,
                    op => return Err(format!("IR は演算子 {op} に未対応").into()),
                };
                if self.raw || !matches!(p, Prim::Add | Prim::Eq | Prim::Ne) {
                    return Ok(prim(p, vec![a, b]));
                }
                let v = call(if p == Prim::Add { "__add" } else { "__eq" }, vec![a, b]);
                if p == Prim::Ne { prim(Prim::Not, vec![v]) } else { v }
            }
            Ast::Call(n, args) => self.call(n, args, id)?,
            Ast::Apply(f, args) => {
                let f = self.expr(f)?;
                let args = self.args(args)?;
                let c = Expr::CallIndirect(Box::new(f), args);
                let c = if self.waits(id) { self.await_(c) } else { c };
                self.checked(c)
            }
            Ast::Field(x, f) => call(&format!("__field_{f}"), vec![self.expr(x)?]),
            Ast::Index(v, i) => {
                let mut stmts = Vec::new();
                let (v, i) = (self.look(v)?, self.expr(i)?);
                let at = add(add(v, int(8)), prim(Prim::Mul, vec![i, int(16)]));
                let at = self.temp(at, &mut stmts);
                block(stmts, getv(at))
            }
            Ast::Array(items) => {
                let items = self.args(items)?;
                let new = call("__arr_new", vec![int(items.len() as i64)]);
                self.object(ARR, new, 8, items)
            }
            Ast::If(c, t, f) => {
                let c = self.expr(c)?;
                let t = self.block(t)?;
                let f = match f {
                    Some(f) => match self.expr(f)? {
                        Expr::Block(b) => *b,
                        value => *leaf(value),
                    },
                    None => *leaf(unit()),
                };
                Expr::If(Box::new(c), Box::new(t), Box::new(f))
            }
            Ast::Block(b) => Expr::Block(Box::new(self.block(b)?)),
            Ast::Match(s, arms) => {
                let mut stmts = Vec::new();
                let s = self.expr(s)?;
                let s = self.temp(s, &mut stmts);
                let ctor = self.local();
                stmts.push(Stmt::Set(ctor, prim(Prim::Load, vec![s.clone()])));
                let mut rest = unit();
                for arm in arms.iter().rev() {
                    let saved = self.vars.clone();
                    let mut binds = Vec::new();
                    for (i, b) in arm.binds.iter().enumerate() {
                        if b != "_" {
                            let x = self.local();
                            binds.push(Stmt::Set(x, getv(add(s.clone(), int(16 + 16 * i as i64)))));
                            self.vars.insert(b.clone(), x);
                        }
                    }
                    let body = Box::new(Block {
                        stmts: binds,
                        value: self.expr(&arm.body)?,
                    });
                    self.vars = saved;
                    if arm.ctor == "_" {
                        rest = Expr::Block(body);
                        continue;
                    }
                    let (k, _) = self
                        .cx
                        .ctors
                        .get(arm.ctor.as_str())
                        .ok_or_else(|| format!("IR は腕 {} に未対応", arm.ctor))?;
                    let cond = prim(Prim::Eq, vec![Expr::Local(ctor), int(*k)]);
                    rest = Expr::If(Box::new(cond), body, leaf(rest));
                }
                block(stmts, rest)
            }
            Ast::Lam(params, _, _, body) => {
                let params: Vec<Name> = params.iter().map(|(p, _)| p.clone()).collect();
                self.lambda(&params, &[], body)?
            }
            Ast::Pi(..) => mkv(TY, Expr::Str("fn".into())),
            Ast::Refine(..) => mkv(TY, Expr::Str("refine".into())),
            Ast::Assign(..) | Ast::While(..) | Ast::For(..) | Ast::Break | Ast::Continue | Ast::Return(_) => {
                let mut stmts = Vec::new();
                self.effect(e, &mut stmts)?;
                block(stmts, unit())
            }
            Ast::Handle(node, body, arms) => self.handle(*node, body, arms)?,
            // exit は、行き先の with の番号に中断の印を置き、呼び出しの後の検査と同じく 1 段ずつ戻る
            Ast::Exit(target, x) => {
                let v = self.expr(x)?;
                let h = self.vars[&exit_name(target.expect("型検査が行き先を書き込む"))];
                self.checked(call("__exit", vec![Expr::Local(h), v]))
            }
            _ => return Err("IR はこの式に未対応".into()),
        })
    }

    fn call(&mut self, n: &Name, args: &[Ast], id: Option<parser::NodeId>) -> R<Expr> {
        if let Some(i) = self.vars.get(n) {
            let i = *i;
            let args = self.args(args)?;
            let c = Expr::CallIndirect(Box::new(Expr::Local(i)), args);
            let c = if self.waits(id) { self.await_(c) } else { c };
            return Ok(self.checked(c));
        }
        // len は読むだけなので、変数の所有の印を残す
        let mut args = if n == "len" && !self.cx.sources.contains_key("len") {
            args.iter().map(|a| self.look(a)).collect::<R<_>>()?
        } else {
            self.args(args)?
        };
        if n == "#num" { return Ok(call("__num", args)); }
        if let Some(p) = intrinsic(n) {
            return Ok(match n.as_str() {
                "__getv" => {
                    let mut stmts = Vec::new();
                    let a = self.temp(args.remove(0), &mut stmts);
                    block(stmts, getv(a))
                }
                "__setv" => {
                    let mut stmts = Vec::new();
                    let a = self.temp(args.remove(0), &mut stmts);
                    let v = self.temp(args.remove(0), &mut stmts);
                    stmts.extend(setv(a, v));
                    block(stmts, unit())
                }
                // __mkv(番号, 中身)
                "__mkv" => prim(Prim::MakeV, args),
                _ => prim(p.unwrap(), args),
            });
        }
        if let Some((i, fields)) = self.cx.ctors.get(n.as_str()) {
            let (i, nf) = (*i, fields.len() as i64);
            let new = call("__data_new", vec![int(i), int(nf)]);
            return Ok(self.object(DATA, new, 16, args));
        }
        if let Some(src) = self.cx.sources.get(n.as_str()) {
            let f = src.f;
            // 操作は、引数を Vec にして内側の handle から腕を探す。推論した型の引数は渡さない
            if f.op.is_some() {
                let k = args.len() - (f.params.len() - f.implicit);
                let items = args.split_off(k);
                let new = call("__arr_new", vec![int(items.len() as i64)]);
                let items = self.object(ARR, new, 8, items);
                // メソッドの宣言に with Async があれば、腕の Promise を待つ（js.rs の $aop）
                if has_async(&f.effects) {
                    let c = self.await_(call("__aperform", vec![Expr::Str(n.to_string()), items]));
                    return Ok(self.checked(c));
                }
                return Ok(self.checked(call("__perform", vec![Expr::Str(n.to_string()), items])));
            }
            // 推論したジェネリック引数は実行時には要らないので unit で埋める
            if args.len() + src.f.implicit == src.f.params.len() {
                let mut full: Vec<Expr> = (0..src.f.implicit).map(|_| unit()).collect();
                full.append(&mut args);
                args = full;
            }
            // async 関数は呼んで待つ。要求の多相な関数を Async で使う所は、async 版を呼んで待つ（js.rs と同じ）
            let poly = !src.runtime
                && id.is_some_and(|id| self.cx.info.async_inst.contains(&id) || (self.variant && is_poly(f) && self.cx.info.var_awaits.contains(&id)));
            if poly {
                let c = self.await_(call(&format!("{n}$a"), args));
                return Ok(self.checked(c));
            }
            let c = call(n, args);
            if !src.runtime && has_async(&f.effects) {
                let c = self.await_(c);
                return Ok(self.checked(c));
            }
            // ponytail: 非同期処理があるプログラムでは、要求の無い関数の呼び出しも検査する。負荷があれば、中断しうる関数を呼び出しの関係から絞る
            return Ok(if self.cx.panic_checks || !f.effects.is_empty() { self.checked(c) } else { c });
        }
        if matches!(n.as_str(), "Vec" | "Type" | "Ref" | "Async") || self.cx.types.contains(n.as_str()) {
            return Ok(mkv(TY, Expr::Str(n.to_string())));
        }
        // with の腕の #resume は、ランタイムの __resume
        let rt = if n == "#resume" { "__resume".to_string() } else { format!("__{n}") };
        if let Some(s) = self.cx.sources.get(rt.as_str()).filter(|s| s.runtime) {
            // 省いた引数は unit で埋める
            while args.len() < s.f.params.len() {
                args.push(unit());
            }
            let c = Expr::Call(rt, args);
            // all は Promise.all を待つ async 関数（js.rs の $all）
            return Ok(match n.as_str() {
                "all" if !self.raw => {
                    let c = self.await_(c);
                    self.checked(c)
                }
                "panic" if self.cx.panic_checks => self.checked(c),
                _ => c,
            });
        }
        // 変換の中で作る関数（__ctor_name など）。無ければ作る時に誤りにする
        if self.raw && n.starts_with("__") {
            return Ok(call(n, args));
        }
        Err(format!("IR は関数 {n} の呼び出しに未対応").into())
    }

    // 無名関数。本体が使う局所変数（exit の行き先を含む）を写し取り、別の関数にする
    // unpack は handle の腕の束縛。腕は引数の Vec を 1 つ受け取り、そこから取り出す
    fn lambda(&mut self, params: &[Name], unpack: &[Name], body: &parser::Block) -> R<Expr> {
        let mut used = HashSet::new();
        crate::typeck::walk(&Ast::Block(body.clone()), &mut |e| match e {
            Ast::Var(n) | Ast::Call(n, _) | Ast::Assign(n, _) => {
                used.insert(n.clone());
            }
            Ast::Exit(Some(t), _) => {
                used.insert(exit_name(*t));
            }
            _ => {}
        });
        let mut caps: Vec<(Name, u32)> = self.vars.iter().filter(|(n, _)| used.contains(*n)).map(|(n, i)| (n.clone(), *i)).collect();
        caps.sort();
        self.cx.lambdas += 1;
        let name = format!("__lam{}", self.cx.lambdas);
        let mut l = Lower {
            cx: &mut *self.cx,
            vars: HashMap::new(),
            locals: 1,
            raw: self.raw,
            handles: Vec::new(),
            loop_handle_depth: None,
            muts: HashSet::new(),
            variant: self.variant,
        };
        let mut stmts = Vec::new();
        // 引数は 1 番から並べる。名前は写し取った変数より後に入れて、外の同じ名前を隠す
        let param_locals: Vec<u32> = params.iter().map(|_| l.local()).collect();
        for (k, (n, _)) in caps.iter().enumerate() {
            let i = l.local();
            stmts.push(Stmt::Set(
                i,
                getv(add(Expr::Local(0), int(16 + 16 * k as i64))),
            ));
            l.vars.insert(n.clone(), i);
        }
        for (p, i) in params.iter().zip(param_locals) {
            l.vars.insert(p.clone(), i);
        }
        for (k, b) in unpack.iter().enumerate() {
            if b != "_" {
                let i = l.local();
                stmts.push(Stmt::Set(
                    i,
                    getv(add(Expr::Local(1), int(8 + 16 * k as i64))),
                ));
                l.vars.insert(b.clone(), i);
            }
        }
        let b = l.block(body)?;
        stmts.extend(b.stmts);
        stmts.push(Stmt::Return(b.value));
        let locals = l.locals;
        let mut function = Function {
            name: name.clone(),
            params: 1 + params.len() as u32,
            param_types: vec![],
            result_type: ValueType::Other,
            finish_at_boundary: false,
            locals,
            body: stmts,
        };
        if !self.raw && self.waits(Some(body.id)) {
            let entry = self.cx.coroutine(function);
            self.cx.made.push(entry);
        } else {
            if !self.raw {
                self.cx.shadow(&mut function);
            }
            self.cx.made.push(function);
        }
        let new = call("__fn_new", vec![Expr::Func(name), int(caps.len() as i64)]);
        let items = caps.into_iter().map(|(n, i)| self.shared(&n, i)).collect();
        Ok(self.object(FN, new, 16, items))
    }

    // with を書き換えた Handle。番号を先に決めて腕（無名関数）に写し取らせ（exit の行き先）、腕を登録し、本体は Catch の中で評価する
    fn handle(&mut self, node: parser::NodeId, body: &Ast, arms: &[parser::Arm]) -> R<Expr> {
        let h = self.local();
        let mut stmts = vec![Stmt::Set(h, call("__handle_id", vec![]))];
        self.vars.insert(exit_name(node), h);
        let mut ops = Vec::new();
        let mut fns = Vec::new();
        for a in arms {
            ops.push(Expr::Str(a.ctor.to_string()));
            // 本体のブロックの識別子は残す（async な腕かを型検査が記録している）
            let b = match &a.body {
                Ast::Block(b) => b.clone(),
                body => parser::Block { id: Default::default(), stmts: vec![], tail: Some(Box::new(body.clone())) },
            };
            fns.push(self.lambda(&[Name::from("#args")], &a.binds, &b)?);
        }
        let n = int(ops.len() as i64);
        self.vars.remove(&exit_name(node));
        let ops = self.object(ARR, call("__arr_new", vec![n.clone()]), 8, ops);
        let fns = self.object(ARR, call("__arr_new", vec![n]), 8, fns);
        stmts.push(Stmt::Eval(call("__handle_push", vec![Expr::Local(h), ops, fns])));
        self.handles.push(h);
        let v = self.expr(body);
        self.handles.pop();
        let r = self.local();
        stmts.push(Stmt::Set(r, Expr::Catch(leaf(v?))));
        let end = call("__handle_end", vec![Expr::Local(h), Expr::Local(r)]);
        // 外側の handle への中断なら、さらに外へ伝える
        let end = if self.raw { end } else { self.checked(end) };
        Ok(block(stmts, end))
    }
}

// runtime.hb で使う組み込み。Some(None) は変換の中で組み立てるもの
fn intrinsic(n: &str) -> Option<Option<Prim>> {
    Some(Some(match n {
        "__tag" => Prim::Tag,
        "__payload" => Prim::Payload,
        "__load" => Prim::Load,
        "__store" => Prim::Store,
        "__load8" => Prim::Load8,
        "__store8" => Prim::Store8,
        "__pages" => Prim::Pages,
        "__static" => Prim::Static,
        "__sys_write" => Prim::Write,
        "__fail" => Prim::Fail,
        "__now" => Prim::Now,
        "__sys_read" => Prim::Read,
        "__f_from_int" => Prim::FFromInt,
        "__f_to_int" => Prim::FToInt,
        "__f_add" => Prim::FAdd,
        "__f_sub" => Prim::FSub,
        "__f_mul" => Prim::FMul,
        "__f_div" => Prim::FDiv,
        "__f_neg" => Prim::FNeg,
        "__f_floor" => Prim::FFloor,
        "__f_sqrt" => Prim::FSqrt,
        "__f_eq" => Prim::FEq,
        "__f_lt" => Prim::FLt,
        "__f_le" => Prim::FLe,
        "__and" => Prim::And,
        "__or" => Prim::Or,
        "__shl" => Prim::Shl,
        "__shr" => Prim::Shr,
        "__getv" | "__setv" | "__mkv" => return Some(None),
        _ => return None,
    }))
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for fun in self.functions.iter().filter(|f| !f.name.starts_with("__")) {
            write!(f, "fn {}(", fun.name)?;
            for i in 0..fun.params {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "%{i}")?;
            }
            writeln!(f, ") {{")?;
            stmts(f, &fun.body, 1)?;
            writeln!(f, "}}")?;
        }
        Ok(())
    }
}

fn stmts(f: &mut fmt::Formatter<'_>, ss: &[Stmt], depth: usize) -> fmt::Result {
    let pad = "  ".repeat(depth);
    for s in ss {
        match s {
            Stmt::Set(i, e) => writeln!(f, "{pad}%{i} = {e}")?,
            Stmt::Eval(e) => writeln!(f, "{pad}{e}")?,
            Stmt::Return(e) => writeln!(f, "{pad}return {e}")?,
            Stmt::Break => writeln!(f, "{pad}break")?,
            Stmt::Continue => writeln!(f, "{pad}continue")?,
            Stmt::Escape => writeln!(f, "{pad}escape")?,
            Stmt::While(c, body) => {
                writeln!(f, "{pad}while {c} {{")?;
                stmts(f, body, depth + 1)?;
                writeln!(f, "{pad}}}")?;
            }
        }
    }
    Ok(())
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.stmts.is_empty() {
            return write!(f, "{}", self.value);
        }
        writeln!(f, "{{")?;
        stmts(f, &self.stmts, 2)?;
        write!(f, "    {} }}", self.value)
    }
}

fn list(f: &mut fmt::Formatter<'_>, head: &str, args: &[Expr]) -> fmt::Result {
    write!(f, "({head}")?;
    for a in args {
        write!(f, " {a}")?;
    }
    f.write_str(")")
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expr::Const(INT, n) => write!(f, "{n}"),
            Expr::Const(UNIT, _) => write!(f, "()"),
            Expr::Const(BOOL, b) => write!(f, "{}", *b != 0),
            Expr::Const(t, n) => write!(f, "(const {t} {n})"),
            Expr::Str(s) => write!(f, "{s:?}"),
            Expr::Local(i) => write!(f, "%{i}"),
            Expr::Func(n) => write!(f, "(func {n})"),
            Expr::Prim(p, args) => list(f, &format!("{p:?}"), args),
            Expr::Call(n, args) => list(f, &format!("call {n}"), args),
            Expr::CallIndirect(g, args) => list(f, &format!("call_indirect {g}"), args),
            Expr::If(c, t, e) => write!(f, "(if {c} {t} {e})"),
            Expr::Block(b) => write!(f, "{b}"),
            Expr::Catch(b) => write!(f, "(catch {b})"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lowers_checked_program_in_evaluation_order() {
        let src = "fn add(x: int, y: int) -> int { x - y }\nfn main() { let x = add(1, 2)\n let x = x * 3\n x / 4; }";
        let ir = crate::to_ir(src).unwrap();
        assert_eq!(
            ir,
            // 局所変数は入口で確保したシャドースタックの区画（%2）にも書き、戻る前に区画を外す
            "fn add(%0, %1) {\n  %2 = (call __frame_push 2)\n  (Store (Add %2 0) (Tag %0))\n  (Store (Add (Add %2 0) 8) (Payload %0))\n  (Store (Add %2 16) (Tag %1))\n  (Store (Add (Add %2 16) 8) (Payload %1))\n  (if (Gt (Load (Add (Static) 96)) (Load (Add (Static) 104))) {\n    (call __gc)\n    () } ())\n  %3 = (call __isub %0 %1)\n  (Store (Add (Static) 64) %2)\n  return %3\n}\nfn main() {\n  %2 = (call __frame_push 2)\n  (if (Gt (Load (Add (Static) 96)) (Load (Add (Static) 104))) {\n    (call __gc)\n    () } ())\n  %0 = (call add 1 2)\n  (Store (Add %2 0) (Tag %0))\n  (Store (Add (Add %2 0) 8) (Payload %0))\n  %1 = (call __imul %0 3)\n  (Store (Add %2 16) (Tag %1))\n  (Store (Add (Add %2 16) 8) (Payload %1))\n  (Div %1 4)\n  %3 = ()\n  (Store (Add (Static) 64) %2)\n  return %3\n}\n"
        );
    }

    #[test]
    fn lowers_control_flow() {
        let src = "fn f(n: int) -> bool {\n let mut i = 0\n while i < n { if i > 3 { return true }\n i = i - 1 }\n false\n}\nfn main() { put(f(5)) }";
        let ir = crate::to_ir(src).unwrap();
        assert!(ir.contains("while (Lt %1 %0) {"), "{ir}");
        assert!(ir.contains("(if (Gt %1 3) {"), "{ir}");
        assert!(ir.contains("(call __put (call f 5))"), "{ir}");
    }

    #[test]
    fn rejects_unsupported_and_invalid_programs() {
        let src = "extern 'browser' {\n    fn show(r: Ref<int>)\n}\nfn main() { show(new_ref(1)) }";
        assert!(crate::to_ir(src).unwrap_err().contains("未対応"));
        assert!(crate::to_ir("fn main() { let x = 1 + true }").is_err());
    }
}

// with（Handle）の番号を腕に写し取らせる変数の名前
fn exit_name(node: parser::NodeId) -> Name {
    Name::temporary("exit", node.raw() as usize)
}
