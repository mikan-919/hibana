use crate::lexer::{Span, Tok, Token, lex};
use crate::meta::{Grammar, Node, Q};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Debug, Clone)]
#[allow(dead_code)] // Field と Method の中身は型検査が対応するまで読まれない
pub enum Expr {
    Int(i64),
    Bool(bool),
    Str(String),
    Var(String),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    // 式.名前 と 式.名前(引数)。構文のみで型検査は未対応
    Field(Box<Expr>, String),
    Method(Box<Expr>, String, Vec<Expr>),
    Assign(String, Box<Expr>),
    If(Box<Expr>, Block, Option<Box<Expr>>),
    While(Box<Expr>, Block),
    Block(Block),
    Return(Option<Box<Expr>>),
    // 展開コードの無いマクロの照合結果
    Macro(String, Box<Node>),
    // quote { ... }。評価すると syn 型の値になる
    Quote(Vec<Q>),
    Array(Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    // 細分化型 型[変数 -> 条件]
    Refine(Box<Expr>, String, Box<Expr>),
    Match(Box<Expr>, Vec<Arm>),
    // 無名関数 fn(引数) -> 戻り値の型 ! 効果 { 本体 }。引数の型は期待する型から分かれば省ける
    Lam(
        Vec<(String, Option<Expr>)>,
        Option<Box<Expr>>,
        Vec<Expr>,
        Block,
    ),
    // 関数型 (x: A, ...) -> B ! 効果。名前の無い引数は "_"
    Pi(Vec<(String, Expr)>, Box<Expr>, Vec<Expr>),
    // 式の値の関数を呼ぶ
    Apply(Box<Expr>, Vec<Expr>),
    // handle 式 { 操作(引数) => 式 ... }。腕の ctor は操作の名前、binds は引数
    Handle(Box<Expr>, Vec<Arm>),
    // for x in 式 { } と for x in a..b { }
    For(String, Box<Expr>, Option<Box<Expr>>, Block),
    // ソース上の位置。エラー表示に使う
    At(Span, Box<Expr>),
}

impl Expr {
    // 位置の包みを外す
    pub fn peel(&self) -> &Expr {
        match self {
            Expr::At(_, e) => e.peel(),
            e => e,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        mutable: bool,
        ty: Option<Expr>,
        init: Expr,
    },
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}

#[derive(Debug)]
pub struct Fn {
    pub name: String,
    pub params: Vec<(String, Expr)>,
    // None は unit
    pub ret: Option<Expr>,
    pub body: Block,
    // type Name(引数) = 式 で書いた別名。引数が無ければ () を省いて呼べる
    pub alias: bool,
    // 定義したモジュール（利用者のファイルは ""）
    pub module: String,
    // partial fn。停止性を検査しない代わりに、型の計算で展開しない
    pub partial: bool,
    // params の先頭の何個がジェネリック引数か
    pub implicit: usize,
    // extern で宣言した JavaScript の関数。Some(モジュール)、グローバルの関数は Some("")
    pub js: Option<String>,
    // ! の後ろに書いた効果（Fail<E> などの型の式）
    pub effects: Vec<Expr>,
    // effect 宣言の操作なら、その効果の名前
    pub op: Option<String>,
    // params の末尾の何個がトレイトの実装（<A: Show> の制約）か。呼び出しで省くと型検査が実装を探して足す
    pub insts: usize,
}

pub struct Parser<'s> {
    pub toks: Vec<Token>,
    pub pos: usize,
    pub src: &'s str,
    pub g: Rc<Grammar>,
    // マクロ展開の入れ子の深さ
    pub depth: usize,
    // 宣言された型の名前。直後の < を型引数として読む
    pub types: Rc<HashSet<String>>,
    // ジェネリック引数を持つ関数の名前。f<int>(x) の < を型引数として読む
    pub generic_fns: Rc<HashSet<String>>,
    // 大域の名前（関数・型・構築子・組み込み・パターンのリテラル）
    pub globals: Rc<HashSet<String>>,
    // 構造体の名前からフィールド名の並び
    pub structs: Rc<HashMap<String, Vec<String>>>,
    // 戻り値の型を読んでいる間は、続く { を構造体の値と読まない
    pub no_struct: bool,
    // トークンの位置から、そこを含むモジュール（利用者のファイルは ""）
    pub module_at: Rc<Vec<(usize, String)>>,
    // モジュールごとの名前（修飾しない形）
    pub mod_names: Rc<HashMap<String, HashSet<String>>>,
    // モジュールごとの use
    pub uses: Rc<HashMap<String, Vec<Use>>>,
    // use されたモジュールの名前。web.ok の web
    pub modules: Rc<HashSet<String>>,
    // 修飾した形の大域の名前（関数・型・構築子）
    pub known: Rc<HashSet<String>>,
    // トレイトの名前（修飾した形）から、その構造体のフィールドの並び
    pub traits: Rc<HashMap<String, Trait>>,
}

// trait 宣言。トレイトは構造体の型 Name<A> = { super#親: 親<A>, メソッド: 関数型 } にする
#[derive(Debug, Clone, Default)]
pub struct Trait {
    pub supers: Vec<String>,
    // (メソッド名, 引数名, 既定の実装があるか)
    pub methods: Vec<(String, Vec<String>, bool)>,
}

use crate::error::Error;
pub use crate::error::R;

// data 宣言。構築子の ret は添字付きの型族の戻り値の型（無ければ Name(params)）
#[derive(Debug)]
pub struct Data {
    pub name: String,
    pub module: String,
    pub params: Vec<(String, Expr)>,
    pub ctors: Vec<Ctor>,
}

#[derive(Debug)]
pub struct Ctor {
    pub name: String,
    pub fields: Vec<(String, Expr)>,
    pub ret: Option<Expr>,
}

// match の腕。ctor が "_" なら残りすべて
#[derive(Debug, Clone)]
pub struct Arm {
    pub ctor: String,
    pub binds: Vec<String>,
    pub body: Expr,
}

enum TypeDecl {
    Data(Data),
    Alias(Fn),
}

#[derive(Debug, Default)]
pub struct Program {
    pub fns: Vec<Fn>,
    pub datas: Vec<Data>,
    // test "名前" { 本体 } の (名前, 本体の関数名)。hibana test で実行する
    pub tests: Vec<(String, String)>,
}

// 組み込みのライブラリ。実行ファイルに埋め込むので、どこからでも use できる
const LIBS: &[(&str, &str)] = &[
    ("web", include_str!("../lib/web.hb")),
    ("std", include_str!("../lib/std.hb")),
];

// use した名前の取り込み方
#[derive(Debug, Clone)]
pub enum UseNames {
    // use web: web.ok のように修飾して使う
    Qualified,
    // use web.*: すべて修飾なしで使える
    All,
    // use web.{ok, Request}: 挙げた名前だけ修飾なしで使える
    List(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct Use {
    pub module: String,
    pub names: UseNames,
}

// モジュール名 m の中の名前 n の内部での名前。利用者のファイル（m が空）では n のまま
pub fn qual(m: &str, n: &str) -> String {
    if m.is_empty() {
        n.to_string()
    } else {
        format!("{m}.{n}")
    }
}

// use に書いた名前から、モジュール名を作る。ファイルは拡張子を除いた名前（./js/util.ts は util）、
// node:path は path、@scope/pkg は pkg。名前に使えない文字は _ にする
pub fn module_name(spec: &str) -> String {
    let last = spec.rsplit(['/', ':']).next().unwrap_or(spec);
    let stem = last.split('.').next().unwrap_or(last);
    stem.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// use に書いたファイルの経路の起点（use を書いた .hb のディレクトリ）
thread_local! {
    static BASE: std::cell::RefCell<std::path::PathBuf> = std::cell::RefCell::new(".".into());
}

pub fn set_base_dir(p: &std::path::Path) {
    BASE.with(|b| *b.borrow_mut() = p.to_path_buf());
}

fn base_dir() -> std::path::PathBuf {
    BASE.with(|b| b.borrow().clone())
}

// JavaScript のパッケージ（npm・node:）の型宣言 .d.ts を node_modules から探す
fn find_dts(spec: &str) -> Option<String> {
    let nm = base_dir().join("node_modules");
    let mut cands = Vec::new();
    if let Some(m) = spec.strip_prefix("node:") {
        cands.push(nm.join("@types/node").join(format!("{m}.d.ts")));
    } else {
        let pkg = nm.join(spec);
        if let Ok(j) = std::fs::read_to_string(pkg.join("package.json")) {
            // package.json の "types" か "typings"
            for key in ["\"types\"", "\"typings\""] {
                if let Some(rest) = j.split(key).nth(1)
                    && let Some(v) = rest.split('"').nth(1)
                {
                    cands.push(pkg.join(v));
                }
            }
        }
        cands.push(pkg.join("index.d.ts"));
        cands.push(nm.join("@types").join(spec).join("index.d.ts"));
    }
    cands
        .into_iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
}

// use の対象のモジュール名と本文。組み込みのライブラリ、.hb のファイル、JavaScript / TypeScript のモジュール
// use を読み込んだ結果 (モジュール名, ソース, そのソースの use "x.hb" の起点)
fn load(name: &str, is_str: bool, dir: &std::path::Path, span: crate::lexer::Span) -> R<(String, String, std::path::PathBuf)> {
    // use web のように名前で書いたら組み込みのライブラリか hibana.toml の依存、文字列で書いたらファイルか npm
    if !is_str {
        if let Some((_, t)) = LIBS.iter().find(|l| l.0 == name) {
            return Ok((name.to_string(), t.to_string(), dir.to_path_buf()));
        }
        let Some(e) = crate::pkg::entry(&base_dir(), name) else {
            return Err(Error::from(format!("ライブラリ {name} が無い"))
                .at(span)
                .help("パッケージなら hibana.toml の [dependencies] に書き、hibana fetch で取ってくる".into()));
        };
        let text = std::fs::read_to_string(&e)
            .map_err(|err| Error::from(format!("{}: {err}（hibana fetch で取ってくる）", e.display())).at(span))?;
        return Ok((name.to_string(), text, e.parent().unwrap().to_path_buf()));
    }
    let read = |p: &str| {
        std::fs::read_to_string(dir.join(p))
            .map_err(|e| Error::from(format!("{p}: {e}")).at(span))
    };
    if name.ends_with(".hb") {
        let sub = dir.join(name).parent().unwrap().to_path_buf();
        return Ok((module_name(name), read(name)?, sub));
    }
    // JavaScript / TypeScript は型宣言から extern の宣言を作る（JS ターゲットでだけ使える）
    let dts = if name.ends_with(".ts") {
        read(name)?
    } else if let Some(base) = name.strip_suffix(".js").or(name.strip_suffix(".mjs")) {
        read(&format!("{base}.d.ts"))
            .map_err(|_| Error::from(format!("{name} の型宣言 {base}.d.ts が無い")).at(span))?
    } else if name.starts_with("node:") || !name.contains('/') || name.starts_with('@') {
        find_dts(name).ok_or_else(|| {
            Error::from(format!("{name} の型宣言（.d.ts）が node_modules に無い（node: の組み込みには @types/node が要る）"))
                .at(span)
        })?
    } else {
        return Err(Error::from(format!("ライブラリ {name} が無い")).at(span));
    };
    let text = crate::ts::module_text(&dts, name).map_err(|e| e.at(span))?;
    Ok((module_name(name), text, dir.to_path_buf()))
}

// use したモジュールの本文を、`module 名前` を前に付けてソースの後ろに足す。同じものは1回だけ読む
pub fn with_uses(src: &str) -> R<String> {
    let mut out = src.to_string();
    let mut done: Vec<String> = Vec::new();
    let mut todo = vec![(src.to_string(), base_dir())];
    while let Some((s, dir)) = todo.pop() {
        let toks = lex(&s)?;
        for w in toks.windows(2) {
            let (name, is_str) = match (&w[0].tok, &w[1].tok) {
                (Tok::Punct("use"), Tok::Ident(n) | Tok::Str(n)) => {
                    (n.clone(), matches!(w[1].tok, Tok::Str(_)))
                }
                (Tok::Punct("use"), _) => {
                    return Err(
                        Error::from("use の後にはライブラリ名かファイル名が必要").at(w[1].span)
                    );
                }
                _ => continue,
            };
            if done.contains(&name) {
                continue;
            }
            // ponytail: 別のパッケージにある同じ名前の "x.hb" は一つ目だけ読む。名前の衝突はパッケージごとのモジュールにするときに直す
            let (module, text, sub) = load(&name, is_str, &dir, w[1].span)?;
            done.push(name);
            out += &format!("\nmodule {module}\n");
            out += &text;
            todo.push((text, sub));
        }
    }
    Ok(out)
}

pub fn parse(src: &str) -> R<Program> {
    let mut p = Parser {
        toks: lex(src)?,
        pos: 0,
        src,
        g: Rc::default(),
        depth: 0,
        types: Rc::default(),
        generic_fns: Rc::default(),
        globals: Rc::default(),
        structs: Rc::default(),
        no_struct: false,
        module_at: Rc::default(),
        mod_names: Rc::default(),
        uses: Rc::default(),
        modules: Rc::default(),
        known: Rc::default(),
        traits: Rc::default(),
    };
    let mut g = Grammar::default();
    let mut fn_starts = Vec::new();
    let mut data_starts = Vec::new();
    let mut extern_starts = Vec::new();
    let mut effect_starts = Vec::new();
    let mut trait_starts = Vec::new();
    let mut test_starts = Vec::new();
    let mut impl_starts = Vec::new();
    let mut types: HashSet<String> = ["Vec", "Ref", "Type"].map(String::from).into();
    let mut generic_fns: HashSet<String> = HashSet::new();
    // 組み込みと利用者のファイルの名前・モジュール名・パターンのリテラル。マクロの衛生性で名前を付け替えない
    let mut globals: HashSet<String> = [
        "js",
        "print",
        "len",
        "at",
        "is_null",
        "int_of",
        "str_of",
        "error",
        "int",
        "bool",
        "str",
        "syn",
        "unit",
        "ident",
        "to_str",
        "split",
        "starts_with",
        "slice",
        "panic",
        "resume",
        "all",
        "push",
        "Ref",
        "new_ref",
        "read",
        "write",
        "Async",
        "Vec",
        "Type",
    ]
    .map(String::from)
    .into();
    // モジュールごとの名前（修飾しない形）と、修飾した形の大域の名前
    let mut mod_names: HashMap<String, HashSet<String>> = HashMap::new();
    let mut known: HashSet<String> = HashSet::new();
    let mut uses: HashMap<String, Vec<Use>> = HashMap::new();
    let mut modules: HashSet<String> = HashSet::new();
    let mut module_at: Vec<(usize, String)> = vec![(0, String::new())];
    let mut declare = |m: &str, n: &str, globals: &mut HashSet<String>| {
        mod_names
            .entry(m.to_string())
            .or_default()
            .insert(n.to_string());
        known.insert(qual(m, n));
        if m.is_empty() {
            globals.insert(n.to_string());
        }
    };
    // 1周目は syntax と macro だけを読む（宣言より前の関数でもマクロを使えるようにする）
    loop {
        let cur = module_at.last().unwrap().1.clone();
        match p.peek() {
            Tok::Eof => break,
            // with_uses が足したモジュールの始まり
            Tok::Punct("module") => {
                p.pos += 1;
                let m = p.ident()?;
                module_at.push((p.pos, m));
                p.module_at = Rc::new(module_at.clone());
            }
            Tok::Punct("use") => {
                let u = p.use_decl()?;
                modules.insert(u.module.clone());
                uses.entry(cur).or_default().push(u);
            }
            Tok::Punct("syntax") => p.syntax_decl(&mut g)?,
            Tok::Punct("macro") => p.macro_decl(&mut g)?,
            // extern "モジュール" { fn 名前(引数) -> 型 ... }
            Tok::Punct("extern") => {
                extern_starts.push(p.pos);
                let start = p.pos;
                p.skip_fn()?;
                for w in p.toks[start..p.pos].windows(2) {
                    if let (Tok::Punct("fn"), Tok::Ident(n)) = (&w[0].tok, &w[1].tok) {
                        let n = n.clone();
                        declare(&cur, &n, &mut globals);
                    }
                }
            }
            Tok::Punct("effect") => {
                if let Some(Tok::Ident(n)) = p.toks.get(p.pos + 1).map(|t| t.tok.clone()) {
                    types.insert(qual(&cur, &n));
                    declare(&cur, &n, &mut globals);
                }
                effect_starts.push(p.pos);
                let start = p.pos;
                p.skip_fn()?;
                for w in p.toks[start..p.pos].windows(2) {
                    if let (Tok::Punct("fn"), Tok::Ident(n)) = (&w[0].tok, &w[1].tok) {
                        let n = n.clone();
                        declare(&cur, &n, &mut globals);
                    }
                }
            }
            Tok::Punct("fn" | "partial") => {
                let k = if p.peek() == &Tok::Punct("partial") {
                    2
                } else {
                    1
                };
                if let Some(Tok::Ident(n)) = p.toks.get(p.pos + k).map(|t| t.tok.clone()) {
                    if p.toks.get(p.pos + k + 1).map(|t| &t.tok) == Some(&Tok::Punct("<")) {
                        generic_fns.insert(qual(&cur, &n));
                    }
                    declare(&cur, &n, &mut globals);
                }
                fn_starts.push(p.pos);
                p.skip_fn()?;
            }
            Tok::Punct("type") => {
                if let Some(Tok::Ident(n)) = p.toks.get(p.pos + 1).map(|t| t.tok.clone()) {
                    types.insert(qual(&cur, &n));
                    declare(&cur, &n, &mut globals);
                }
                data_starts.push(p.pos);
                let start = p.pos;
                p.skip_decl();
                // = や | の直後の名前は構築子（別名なら型の式の先頭だが、どちらも大域の名前なので構わない）
                for w in p.toks[start..p.pos].windows(2) {
                    if let (Tok::Punct("=" | "|"), Tok::Ident(n)) = (&w[0].tok, &w[1].tok) {
                        let n = n.clone();
                        declare(&cur, &n, &mut globals);
                    }
                }
            }
            Tok::Punct("trait") => {
                if let Some(Tok::Ident(n)) = p.toks.get(p.pos + 1).map(|t| t.tok.clone()) {
                    types.insert(qual(&cur, &n));
                    declare(&cur, &n, &mut globals);
                }
                trait_starts.push(p.pos);
                let start = p.pos;
                p.skip_fn()?;
                for w in p.toks[start..p.pos].windows(2) {
                    if let (Tok::Punct("fn"), Tok::Ident(n)) = (&w[0].tok, &w[1].tok) {
                        let n = n.clone();
                        declare(&cur, &n, &mut globals);
                    }
                }
            }
            Tok::Punct("impl") => {
                impl_starts.push(p.pos);
                p.skip_fn()?;
            }
            // test は宣言の先頭でだけ特別な名前（変数名にも使える）
            Tok::Ident(t) if t == "test" && matches!(p.toks.get(p.pos + 1).map(|t| &t.tok), Some(Tok::Str(_))) => {
                test_starts.push(p.pos);
                p.skip_fn()?;
            }
            _ => return p.err("fn・type・effect・trait・impl・extern・syntax・macro・use のいずれかが必要"),
        }
    }
    p.types = Rc::new(types.clone());
    p.generic_fns = Rc::new(generic_fns);
    g.validate()?;
    globals.extend(g.literals());
    globals.extend(modules.iter().cloned());
    p.globals = Rc::new(globals);
    p.mod_names = Rc::new(mod_names);
    p.known = Rc::new(known);
    p.uses = Rc::new(uses);
    p.modules = Rc::new(modules);
    p.module_at = Rc::new(module_at);
    p.g = Rc::new(g);
    p.check_macros()?;
    let mut prog = Program::default();
    for s in effect_starts {
        p.pos = s;
        let (d, ops) = p.effect_decl()?;
        prog.datas.push(d);
        prog.fns.extend(ops);
    }
    for s in data_starts {
        p.pos = s;
        match p.type_decl()? {
            TypeDecl::Data(d) => prog.datas.push(d),
            TypeDecl::Alias(f) => prog.fns.push(f),
        }
    }
    let mut traits = HashMap::new();
    for s in trait_starts {
        p.pos = s;
        let (d, fs, t) = p.trait_decl()?;
        traits.insert(d.name.clone(), t);
        p.traits = Rc::new(traits.clone());
        prog.datas.push(d);
        prog.fns.extend(fs);
    }
    // 構造体の値 Name { ... } をフィールドの順に並べるため、フィールド名を覚えておく
    p.structs = Rc::new(
        prog.datas
            .iter()
            .filter(|d| d.ctors.len() == 1 && d.ctors[0].name.ends_with("{}"))
            .map(|d| {
                (
                    d.name.clone(),
                    d.ctors[0].fields.iter().map(|f| f.0.clone()).collect(),
                )
            })
            .collect(),
    );
    for s in fn_starts {
        p.pos = s;
        prog.fns.push(p.func()?);
    }
    for s in extern_starts {
        p.pos = s;
        prog.fns.extend(p.extern_block()?);
    }
    for (k, s) in impl_starts.into_iter().enumerate() {
        p.pos = s;
        prog.fns.extend(p.impl_decl(k)?);
    }
    // test "名前" { 本体 } は引数の無い関数 test#k にする
    for (k, s) in test_starts.into_iter().enumerate() {
        p.pos = s + 1;
        let Tok::Str(label) = p.next() else { unreachable!() };
        let module = p.cur_module();
        p.expect("{")?;
        let name = qual(&module, &format!("test#{k}"));
        prog.tests.push((label, name.clone()));
        prog.fns.push(Fn {
            name,
            params: vec![],
            ret: None,
            body: p.block()?,
            alias: false,
            module,
            partial: true,
            implicit: 0,
            js: None,
            effects: vec![],
            op: None,
            insts: 0,
        });
    }
    crate::resolve::resolve(&mut prog, &p.uses, &p.known);
    Ok(prog)
}

// a + b + c の項
fn plus_leaves<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    match e.peel() {
        Expr::Binary("+", a, b) => {
            plus_leaves(a, out);
            plus_leaves(b, out);
        }
        _ => out.push(e),
    }
}

// 優先順位（大きいほど強く結合する）
fn binop(t: &Tok) -> Option<(u8, &'static str)> {
    let Tok::Punct(p) = t else { return None };
    let prec = match *p {
        "||" => 1,
        "&&" => 2,
        "==" | "!=" => 3,
        "<" | "<=" | ">" | ">=" => 4,
        "+" | "-" => 5,
        "*" | "/" | "%" => 6,
        _ => return None,
    };
    Some((prec, p))
}

impl Parser<'_> {
    pub fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    pub fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if t != Tok::Eof {
            self.pos += 1;
        }
        t
    }
    pub fn err<T>(&self, msg: &str) -> R<T> {
        self.err_at(format!("{msg}（実際は {:?}）", self.peek()))
    }
    // 今のトークンの位置でエラーにする
    pub fn err_at<T>(&self, msg: String) -> R<T> {
        Err(Error::from(msg).at(self.toks[self.pos].span))
    }
    // 次のトークンが前のトークンと別の行にあるか
    fn at_newline(&self) -> bool {
        self.toks[self.pos].nl_before
    }
    // 文の区切り: ";"、改行、"}" の直前のいずれか
    fn end_stmt(&mut self) -> R<()> {
        if self.eat(";") || self.at_newline() || self.peek() == &Tok::Punct("}") {
            Ok(())
        } else {
            self.err("文の区切り（\";\" か改行）が必要")
        }
    }
    pub fn eat(&mut self, p: &str) -> bool {
        if matches!(self.peek(), Tok::Punct(q) if *q == p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    pub fn expect(&mut self, p: &str) -> R<()> {
        if self.eat(p) {
            Ok(())
        } else {
            self.err(&format!("{p:?} が必要"))
        }
    }
    pub fn ident(&mut self) -> R<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.pos += 1;
                Ok(s)
            }
            _ => self.err("識別子が必要"),
        }
    }
    // 今のトークンが名前なら、修飾した名前に書き換える。web.ok は1個の名前にし、型名は今のモジュールから見た名前にする
    // （構造体の値と <> の判定に使う。それ以外の名前は resolve で局所変数と区別してから解決する）
    fn qualify_here(&mut self) {
        let Tok::Ident(n) = self.peek().clone() else {
            return;
        };
        let at = |k: usize| self.toks.get(self.pos + k).map(|t| t.tok.clone());
        if self.modules.contains(&n)
            && at(1) == Some(Tok::Punct("."))
            && !self.toks[self.pos + 1].gap_before
        {
            if let Some(Tok::Ident(x)) = at(2) {
                self.pos += 2;
                // 巻き戻して読み直した時は、既に書き換えてある
                if !x.contains('.') {
                    self.toks[self.pos].tok = Tok::Ident(format!("{n}.{x}"));
                }
            }
            return;
        }
        let q = self.qualify(&n);
        if q != n
            && (self.types.contains(&q)
                || self.structs.contains_key(&q)
                || self.generic_fns.contains(&q))
        {
            self.toks[self.pos].tok = Tok::Ident(q);
        }
    }

    // 今のトークンを含むモジュール
    pub fn cur_module(&self) -> String {
        self.module_at
            .iter()
            .rev()
            .find(|(i, _)| *i <= self.pos)
            .map(|m| m.1.clone())
            .unwrap_or_default()
    }

    // 修飾していない名前 n を、今のモジュールから見た大域の名前にする。自分のモジュール、利用者のファイル、
    // use で取り込んだ名前の順に探す。見つからなければ n のまま
    pub fn qualify(&self, n: &str) -> String {
        if n.contains('.') {
            return n.to_string();
        }
        let m = self.cur_module();
        let mut cands = vec![qual(&m, n)];
        if m.is_empty() {
            for u in self.uses.get(&m).into_iter().flatten() {
                match &u.names {
                    UseNames::All => cands.push(qual(&u.module, n)),
                    UseNames::List(ns) if ns.iter().any(|x| x == n) => {
                        cands.push(qual(&u.module, n))
                    }
                    _ => {}
                }
            }
        } else {
            for u in self.uses.get(&m).into_iter().flatten() {
                if !matches!(u.names, UseNames::Qualified) {
                    cands.push(qual(&u.module, n));
                }
            }
        }
        let known = |c: &String| self.known.contains(c) || self.types.contains(c);
        cands.into_iter().find(known).unwrap_or(n.to_string())
    }

    // use 名前、use 名前.*、use 名前.{a, b}、use "file.hb"
    fn use_decl(&mut self) -> R<Use> {
        self.expect("use")?;
        let module = match self.next() {
            Tok::Ident(n) => n,
            Tok::Str(f) => module_name(&f),
            _ => return self.err("use の後にはライブラリ名かファイル名が必要"),
        };
        let names = if self.toks[self.pos].tok == Tok::Punct(".") && !self.at_newline() {
            self.pos += 1;
            if self.eat("*") {
                UseNames::All
            } else {
                self.expect("{")?;
                let mut ns = Vec::new();
                while !self.eat("}") {
                    ns.push(self.ident()?);
                    self.eat(",");
                }
                UseNames::List(ns)
            }
        } else {
            UseNames::Qualified
        };
        Ok(Use { module, names })
    }

    // fn f() -> T { の T。続く { は本体
    fn ret_ty(&mut self) -> R<Expr> {
        let saved = std::mem::replace(&mut self.no_struct, true);
        let r = self.ty();
        self.no_struct = saved;
        r
    }

    // 型は式として読む。"=" を代入と取り違えないよう二項演算から読む
    pub fn ty(&mut self) -> R<Expr> {
        self.binary(0)
    }

    fn skip_fn(&mut self) -> R<()> {
        while !self.eat("{") {
            if self.next() == Tok::Eof {
                return self.err("関数の本体が無い");
            }
        }
        let mut depth = 1;
        while depth > 0 {
            match self.next() {
                Tok::Punct("{") => depth += 1,
                Tok::Punct("}") => depth -= 1,
                Tok::Eof => return self.err("\"}\" が足りない"),
                _ => {}
            }
        }
        Ok(())
    }

    fn func(&mut self) -> R<Fn> {
        let partial = self.eat("partial");
        self.expect("fn")?;
        let module = self.cur_module();
        let name = qual(&module, &self.ident()?);
        // ジェネリック引数は引数の先頭に置き、呼び出しで省けるようにする
        let mut params = if self.eat("<") {
            self.type_params()?
        } else {
            vec![]
        };
        // 制約 <A: Show> の実装は引数の末尾に置く
        let insts: Vec<_> = params.extract_if(.., |p| p.0.starts_with("inst#")).collect();
        let implicit = params.len();
        self.expect("(")?;
        params.extend(self.params()?);
        let ret = if self.eat("->") {
            Some(self.ret_ty()?)
        } else {
            None
        };
        let effects = self.effects()?;
        self.expect("{")?;
        let n_insts = insts.len();
        params.extend(insts);
        Ok(Fn {
            name,
            params,
            ret,
            body: self.block()?,
            alias: false,
            module,
            partial,
            implicit,
            js: None,
            effects,
            op: None,
            insts: n_insts,
        })
    }

    // ! 効果, ... 。効果は型の式。引数の並びの中の関数型では、, の後が 名前: なら次の引数
    // ponytail: 名前の無い引数の並び (int, (int) -> int ! Log, str) では str も効果と読むので、関数型を括弧で囲む
    fn effects(&mut self) -> R<Vec<Expr>> {
        let mut es = Vec::new();
        if self.eat("!") {
            loop {
                // Vec<() -> T ! E> の閉じの > を比較と読まないよう、加減乗除より弱い演算子は読まない
                let saved = std::mem::replace(&mut self.no_struct, true);
                let e = self.binary(5);
                self.no_struct = saved;
                es.push(e?);
                let next_param = matches!(
                    self.toks.get(self.pos + 1).map(|t| &t.tok),
                    Some(Tok::Ident(_))
                ) && self.toks.get(self.pos + 2).map(|t| &t.tok)
                    == Some(&Tok::Punct(":"));
                if next_param || !self.eat(",") {
                    break;
                }
            }
        }
        Ok(es)
    }

    // effect Name<引数> { fn 操作(引数) -> 型 ... }。効果は構築子の無い型にする。
    // 操作は本体の無い関数で、効果の引数をジェネリック引数として先頭に持つ
    fn effect_decl(&mut self) -> R<(Data, Vec<Fn>)> {
        self.expect("effect")?;
        let module = self.cur_module();
        let name = qual(&module, &self.ident()?);
        let params = if self.eat("<") {
            self.type_params()?
        } else {
            vec![]
        };
        self.expect("{")?;
        let mut ops = Vec::new();
        while !self.eat("}") {
            self.expect("fn")?;
            let op = qual(&module, &self.ident()?);
            self.expect("(")?;
            let mut ps = params.clone();
            ps.extend(self.params()?);
            let ret = if self.eat("->") {
                Some(self.ret_ty()?)
            } else {
                None
            };
            ops.push(Fn {
                name: op,
                params: ps,
                ret,
                body: Block {
                    stmts: vec![],
                    tail: None,
                },
                alias: false,
                module: module.clone(),
                partial: true,
                implicit: params.len(),
                js: None,
                effects: vec![],
                op: Some(name.clone()),
                insts: 0,
            });
            self.eat(",");
        }
        let d = Data {
            name,
            module,
            params,
            ctors: vec![],
        };
        Ok((d, ops))
    }

    // 名前がトレイトなら、修飾した形
    fn trait_name(&self, c: &str) -> Option<String> {
        if self.traits.contains_key(c) {
            return Some(c.to_string());
        }
        let q = self.qualify(c);
        self.traits.contains_key(&q).then_some(q)
    }

    // trait Name<A>: 親<A> + ... { fn メソッド(引数) -> 型 ! 効果 [{ 既定の実装 }] ... }
    // 構造体の型 Name<A> と、メソッドを呼ぶ関数（実装を末尾の引数 inst#0 で受け取る）・Name#get・既定の実装の関数にする
    // ponytail: メソッドは自分のジェネリック引数を持てない。持たせるならメソッドの関数型を依存関数型にする
    fn trait_decl(&mut self) -> R<(Data, Vec<Fn>, Trait)> {
        self.expect("trait")?;
        let module = self.cur_module();
        let name = qual(&module, &self.ident()?);
        self.expect("<")?;
        let params = self.type_params()?;
        let me = Expr::Call(name.clone(), params.iter().map(|p| Expr::Var(p.0.clone())).collect());
        let mut fields = Vec::new();
        let mut info = Trait::default();
        if self.eat(":") {
            loop {
                let s = self.binary(6)?;
                let Expr::Call(sn, _) = s.peel() else {
                    return self.err("親のトレイトは 名前<引数> で書く");
                };
                let sn = self.qualify(sn);
                fields.push((format!("super#{sn}"), s.clone()));
                info.supers.push(sn);
                if !self.eat("+") {
                    break;
                }
            }
        }
        self.expect("{")?;
        let mut fns = Vec::new();
        let with_inst = |ps: &[(String, Expr)]| {
            let mut v = params.clone();
            v.extend(ps.iter().cloned());
            v.push(("inst#0".to_string(), me.clone()));
            v
        };
        while !self.eat("}") {
            self.expect("fn")?;
            let m = self.ident()?;
            self.expect("(")?;
            let ps = self.params()?;
            let ret = if self.eat("->") {
                self.ret_ty()?
            } else {
                Expr::Var("unit".into())
            };
            let effects = self.effects()?;
            let vars: Vec<Expr> = ps.iter().map(|p| Expr::Var(p.0.clone())).collect();
            let field = Expr::Field(Box::new(Expr::Var("inst#0".into())), m.clone());
            let mk = |name: String, body: Block| Fn {
                name,
                params: with_inst(&ps),
                ret: Some(ret.clone()),
                body,
                alias: false,
                module: module.clone(),
                partial: false,
                implicit: params.len(),
                js: None,
                effects: effects.clone(),
                op: None,
                insts: 1,
            };
            fns.push(mk(
                qual(&module, &m),
                Block { stmts: vec![], tail: Some(Box::new(Expr::Apply(Box::new(field), vars))) },
            ));
            let has = self.eat("{");
            if has {
                let b = self.block()?;
                fns.push(mk(format!("{name}#default#{m}"), b));
            }
            fields.push((m.clone(), Expr::Pi(ps.clone(), Box::new(ret), effects)));
            info.methods.push((m, ps.into_iter().map(|p| p.0).collect(), has));
            self.eat(",");
        }
        fns.push(Fn {
            name: format!("{name}#get"),
            params: with_inst(&[]),
            ret: Some(me.clone()),
            body: Block { stmts: vec![], tail: Some(Box::new(Expr::Var("inst#0".into()))) },
            alias: false,
            module: module.clone(),
            partial: false,
            implicit: params.len(),
            js: None,
            effects: vec![],
            op: None,
            insts: 1,
        });
        let ctor = Ctor { name: format!("{name}{{}}"), fields, ret: None };
        Ok((Data { name, module, params, ctors: vec![ctor] }, fns, info))
    }

    // impl<G: 制約> Trait<型> { fn メソッド(引数) -> 型 { 本体 } ... }。
    // 実装の値を作る関数 Trait#implK（制約の実装を引数に取る）と、メソッドの関数 Trait#implK#メソッド にする
    fn impl_decl(&mut self, k: usize) -> R<Vec<Fn>> {
        self.expect("impl")?;
        let module = self.cur_module();
        let mut generics = if self.eat("<") { self.type_params()? } else { vec![] };
        let cons: Vec<_> = generics.extract_if(.., |p| p.0.starts_with("inst#")).collect();
        let head = self.ret_ty()?;
        let Expr::Call(tn, _) = head.peel() else {
            return self.err("impl の後には トレイト<型> が必要");
        };
        let Some(q) = self.trait_name(tn) else {
            return self.err_at(format!("{tn} はトレイトでない"));
        };
        let tr = self.traits[&q].clone();
        let short = q.rsplit('.').next().unwrap();
        let base = qual(&module, &format!("{short}#impl{k}"));
        self.expect("{")?;
        let mut fns = Vec::new();
        let mut given = HashMap::new();
        while !self.eat("}") {
            let mut f = self.func()?;
            let m = f.name.rsplit('.').next().unwrap().to_string();
            if !tr.methods.iter().any(|x| x.0 == m) {
                return self.err_at(format!("トレイト {q} にメソッド {m} は無い"));
            }
            f.name = format!("{base}#{m}");
            let mut ps = generics.clone();
            ps.append(&mut f.params);
            ps.extend(cons.iter().cloned());
            f.params = ps;
            f.implicit = generics.len();
            f.insts = cons.len();
            given.insert(m, f.name.clone());
            fns.push(f);
        }
        let mut args: Vec<Expr> = tr.supers.iter().map(|s| Expr::Call(format!("{s}#get"), vec![])).collect();
        for (m, ps, has) in &tr.methods {
            let target = match given.get(m) {
                Some(n) => n.clone(),
                None if *has => format!("{q}#default#{m}"),
                None => return self.err_at(format!("{q} のメソッド {m} の実装が無い")),
            };
            let call = Expr::Call(target, ps.iter().map(|p| Expr::Var(p.clone())).collect());
            let body = Block { stmts: vec![], tail: Some(Box::new(call)) };
            args.push(Expr::Lam(ps.iter().map(|p| (p.clone(), None)).collect(), None, vec![], body));
        }
        let mut params = generics.clone();
        params.extend(cons.iter().cloned());
        fns.push(Fn {
            name: base,
            params,
            ret: Some(head),
            body: Block { stmts: vec![], tail: Some(Box::new(Expr::Call(format!("{q}{{}}"), args))) },
            alias: false,
            module,
            // 既定の実装は自分の実装を探して呼ぶので、この関数は自分を呼びうる（値を作るだけで止まる）
            partial: true,
            implicit: generics.len(),
            js: None,
            effects: vec![],
            op: None,
            insts: cons.len(),
        });
        Ok(fns)
    }

    // extern "モジュール" { fn 名前(引数) -> 型 ... }。本体の無い関数として並べる（JS ターゲットでだけ使える）
    fn extern_block(&mut self) -> R<Vec<Fn>> {
        self.expect("extern")?;
        let js = match self.peek().clone() {
            Tok::Str(s) => {
                self.pos += 1;
                s
            }
            _ => String::new(),
        };
        self.expect("{")?;
        let mut fns = Vec::new();
        while !self.eat("}") {
            self.expect("fn")?;
            let module = self.cur_module();
            let name = qual(&module, &self.ident()?);
            self.expect("(")?;
            let params = self.params()?;
            let ret = if self.eat("->") {
                Some(self.ret_ty()?)
            } else {
                None
            };
            let effects = self.effects()?;
            let body = Block {
                stmts: vec![],
                tail: None,
            };
            // 中身が分からないので、停止性は検査せず、型の計算でも展開しない
            fns.push(Fn {
                name,
                params,
                ret,
                body,
                alias: false,
                module,
                partial: true,
                implicit: 0,
                js: Some(js.clone()),
                effects,
                op: None,
                insts: 0,
            });
            self.eat(",");
        }
        Ok(fns)
    }

    // (名前: 型, ...)。"(" は読み終えた状態で呼ぶ
    fn params(&mut self) -> R<Vec<(String, Expr)>> {
        let mut params = Vec::new();
        while !self.eat(")") {
            let n = self.ident()?;
            self.expect(":")?;
            params.push((n, self.ty()?));
            if !self.eat(",") {
                self.expect(")")?;
                break;
            }
        }
        Ok(params)
    }

    // type Name(引数) = | 構築子 | ... は新しい型、type Name(引数) = 式 は別名
    // <名前: 型, ...>。"<" は読み終えた状態で呼ぶ。閉じの > を比較と読まないよう、加減乗除より弱い演算子は読まない
    fn type_params(&mut self) -> R<Vec<(String, Expr)>> {
        let mut params = Vec::new();
        loop {
            let n = self.ident()?;
            // 型を省いた <A> は <A: Type>
            let t = if self.eat(":") {
                self.binary(5)?
            } else {
                Expr::Var("Type".into())
            };
            // <A: Show + Eq> はトレイトの制約。A は型で、実装を inst#k の引数で受け取る
            let mut leaves = Vec::new();
            plus_leaves(&t, &mut leaves);
            let traits: Option<Vec<String>> = leaves
                .iter()
                .map(|l| match l.peel() {
                    Expr::Var(c) if self.trait_name(c).is_some() => self.trait_name(c),
                    _ => None,
                })
                .collect();
            if let Some(ts) = traits {
                params.push((n.clone(), Expr::Var("Type".into())));
                for c in ts {
                    let k = params.iter().filter(|p| p.0.starts_with("inst#")).count();
                    params.push((format!("inst#{k}"), Expr::Call(c, vec![Expr::Var(n.clone())])));
                }
            } else {
                params.push((n, t));
            }
            if !self.eat(",") {
                self.expect(">")?;
                return Ok(params);
            }
        }
    }

    fn type_decl(&mut self) -> R<TypeDecl> {
        self.expect("type")?;
        let module = self.cur_module();
        let name = qual(&module, &self.ident()?);
        let params = if self.eat("<") {
            self.type_params()?
        } else {
            vec![]
        };
        self.expect("=")?;
        // type Name = { フィールド: 型, ... } は構造体。構築子 Name{} を1つ持つ型として扱う
        if self.eat("{") {
            let mut fields = Vec::new();
            while !self.eat("}") {
                let n = self.ident()?;
                self.expect(":")?;
                fields.push((n, self.ty()?));
                self.eat(",");
            }
            let ctor = Ctor {
                name: format!("{name}{{}}"),
                fields,
                ret: None,
            };
            return Ok(TypeDecl::Data(Data {
                name,
                module,
                params,
                ctors: vec![ctor],
            }));
        }
        if !self.has_bar() {
            let e = self.ty()?;
            let body = Block {
                stmts: vec![],
                tail: Some(Box::new(e)),
            };
            let ret = Some(Expr::Var("Type".into()));
            return Ok(TypeDecl::Alias(Fn {
                name,
                params,
                ret,
                body,
                alias: true,
                module,
                partial: false,
                js: None,
                implicit: 0,
                effects: vec![],
                op: None,
                insts: 0,
            }));
        }
        self.eat("|");
        let mut ctors = Vec::new();
        loop {
            let name = self.ident()?;
            let fields = if self.eat("(") {
                self.params()?
            } else {
                vec![]
            };
            let ret = if self.eat("->") {
                Some(self.ty()?)
            } else {
                None
            };
            ctors.push(Ctor {
                name: qual(&module, &name),
                fields,
                ret,
            });
            if !self.eat("|") {
                break;
            }
        }
        Ok(TypeDecl::Data(Data {
            name,
            module,
            params,
            ctors,
        }))
    }

    // 宣言の終わり（次の宣言の始まり）までに、括弧の外の | があるか
    fn has_bar(&self) -> bool {
        let mut depth = 0;
        for t in &self.toks[self.pos..] {
            match &t.tok {
                Tok::Punct("(" | "[" | "{") => depth += 1,
                Tok::Punct(")" | "]" | "}") => depth -= 1,
                Tok::Punct("|") if depth == 0 => return true,
                Tok::Punct(
                    "fn" | "partial" | "type" | "syntax" | "macro" | "use" | "module" | "extern"
                    | "effect" | "trait" | "impl",
                )
                | Tok::Eof
                    if depth == 0 =>
                {
                    return false;
                }
                _ => {}
            }
        }
        false
    }

    // 次の宣言の始まりまで読み飛ばす
    fn skip_decl(&mut self) {
        self.pos += 1;
        let mut depth = 0;
        loop {
            match self.peek() {
                Tok::Eof => return,
                Tok::Punct("(" | "[" | "{") => depth += 1,
                Tok::Punct(")" | "]" | "}") => depth -= 1,
                Tok::Punct(
                    "fn" | "partial" | "type" | "syntax" | "macro" | "use" | "module" | "extern"
                    | "effect" | "trait" | "impl",
                ) if depth == 0 => {
                    return;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn let_stmt(&mut self) -> R<Stmt> {
        self.expect("let")?;
        let mutable = self.eat("mut");
        let name = self.ident()?;
        let ty = if self.eat(":") {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect("=")?;
        let init = self.expr()?;
        self.end_stmt()?;
        Ok(Stmt::Let {
            name,
            mutable,
            ty,
            init,
        })
    }

    // マクロのパターンの Stmt 用
    pub fn stmt(&mut self) -> R<Stmt> {
        if let Some(e) = self.try_macros(true)? {
            self.eat(";");
            return Ok(Stmt::Expr(e));
        }
        if self.peek() == &Tok::Punct("let") {
            return self.let_stmt();
        }
        let e = self.expr()?;
        self.eat(";");
        Ok(Stmt::Expr(e))
    }

    // "{" は読み終えた状態で呼ぶ
    pub fn block(&mut self) -> R<Block> {
        let mut stmts = Vec::new();
        loop {
            if self.eat("}") {
                return Ok(Block { stmts, tail: None });
            }
            if let Some(e) = self.try_macros(true)? {
                self.end_stmt()?;
                stmts.push(Stmt::Expr(e));
                continue;
            }
            if self.peek() == &Tok::Punct("let") {
                stmts.push(self.let_stmt()?);
                continue;
            }
            let e = self.expr()?;
            if self.eat(";") {
                stmts.push(Stmt::Expr(e));
            } else if self.eat("}") {
                return Ok(Block {
                    stmts,
                    tail: Some(Box::new(e)),
                });
            } else {
                self.end_stmt()?;
                stmts.push(Stmt::Expr(e));
            }
        }
    }

    pub fn expr(&mut self) -> R<Expr> {
        let after = self.toks.get(self.pos + 1).map(|t| &t.tok);
        if let (Tok::Ident(name), Some(Tok::Punct("="))) = (self.peek().clone(), after) {
            let start = self.pos;
            self.pos += 2;
            let e = Expr::Assign(name, Box::new(self.expr()?));
            return Ok(self.wrap(start, e));
        }
        self.binary(0)
    }

    fn binary(&mut self, min: u8) -> R<Expr> {
        let start = self.pos;
        let e = self.unary()?;
        let mut lhs = self.wrap(start, e);
        while let Some((prec, op)) = binop(self.peek()) {
            // 行頭の演算子は前の行に続けない（"-x" を新しい文として読むため）
            if prec < min || self.at_newline() {
                break;
            }
            self.pos += 1;
            let rhs = self.binary(prec + 1)?;
            lhs = self.wrap(start, Expr::Binary(op, Box::new(lhs), Box::new(rhs)));
        }
        Ok(lhs)
    }

    fn wrap(&self, start: usize, e: Expr) -> Expr {
        Expr::At(self.span_from(start), Box::new(e))
    }

    fn unary(&mut self) -> R<Expr> {
        for op in ["-", "!"] {
            if self.eat(op) {
                return Ok(Expr::Unary(op, Box::new(self.unary()?)));
            }
        }
        self.primary()
    }

    // "(" は読み終えた状態で呼ぶ
    fn args(&mut self) -> R<Vec<Expr>> {
        let mut args = Vec::new();
        while !self.eat(")") {
            args.push(self.expr()?);
            if !self.eat(",") {
                self.expect(")")?;
                break;
            }
        }
        Ok(args)
    }

    fn primary(&mut self) -> R<Expr> {
        // マクロをコア構文より先に試す
        if let Some(e) = self.try_macros(false)? {
            return Ok(e);
        }
        self.qualify_here();
        let mut e = match self.next() {
            Tok::Int(n) => Expr::Int(n),
            Tok::Str(s) => Expr::Str(s),
            Tok::Punct("true") => Expr::Bool(true),
            Tok::Punct("false") => Expr::Bool(false),
            // 型の名前に続く < は型引数。比較の < と区別する
            Tok::Ident(name)
                if self.types.contains(&name)
                    && !self.at_newline()
                    && self.peek() == &Tok::Punct("(") =>
            {
                self.pos -= 1;
                return self.err_at(format!("型 {name} には <> で引数を渡す"));
            }
            // 構造体の値 Name { フィールド: 式, ... }。構築子 Name{} の呼び出しにする
            Tok::Ident(name)
                if self.structs.contains_key(&name)
                    && !self.no_struct
                    && !self.at_newline()
                    && self.peek() == &Tok::Punct("{") =>
            {
                self.pos += 1;
                let mut given = Vec::new();
                while !self.eat("}") {
                    let f = self.ident()?;
                    self.expect(":")?;
                    given.push((f, self.expr()?));
                    self.eat(",");
                }
                let order = self.structs[&name].clone();
                let mut args = Vec::new();
                for f in &order {
                    let Some(i) = given.iter().position(|g| &g.0 == f) else {
                        return self.err_at(format!("構造体 {name} のフィールド {f} が無い"));
                    };
                    args.push(given.remove(i).1);
                }
                if let Some((f, _)) = given.first() {
                    return self.err_at(format!("構造体 {name} にフィールド {f} は無い"));
                }
                Expr::Call(format!("{name}{{}}"), args)
            }
            Tok::Ident(name) if self.types.contains(&name) && self.peek() == &Tok::Punct("<") => {
                self.pos += 1;
                let mut args = Vec::new();
                loop {
                    // 閉じの > を比較と読まないよう、加減乗除より弱い演算子は読まない
                    args.push(self.binary(5)?);
                    if !self.eat(",") {
                        self.expect(">")?;
                        break;
                    }
                }
                Expr::Call(name, args)
            }
            Tok::Punct("[") => {
                let mut items = Vec::new();
                while !self.eat("]") {
                    items.push(self.expr()?);
                    if !self.eat(",") {
                        self.expect("]")?;
                        break;
                    }
                }
                Expr::Array(items)
            }
            // f<型, ...>(引数) は型引数を明示した呼び出し。型引数を引数の先頭に並べる
            Tok::Ident(name)
                if self.generic_fns.contains(&name) && self.peek() == &Tok::Punct("<") =>
            {
                self.pos += 1;
                let mut args = Vec::new();
                loop {
                    args.push(self.binary(5)?);
                    if !self.eat(",") {
                        self.expect(">")?;
                        break;
                    }
                }
                self.expect("(")?;
                args.extend(self.args()?);
                Expr::Call(name, args)
            }
            Tok::Ident(name) => {
                if !self.at_newline() && self.eat("(") {
                    Expr::Call(name, self.args()?)
                } else {
                    Expr::Var(name)
                }
            }
            Tok::Punct("(") => {
                // (x: A, B) -> C は関数型。名前の無い1個だけなら括弧で囲んだ式
                let mut entries = Vec::new();
                while !self.eat(")") {
                    let after = self.toks.get(self.pos + 1).map(|t| &t.tok);
                    let name = match (self.peek().clone(), after) {
                        (Tok::Ident(n), Some(Tok::Punct(":"))) => {
                            self.pos += 2;
                            Some(n)
                        }
                        _ => None,
                    };
                    entries.push((name, self.expr()?));
                    if !self.eat(",") {
                        self.expect(")")?;
                        break;
                    }
                }
                if self.eat("->") {
                    let binders = entries
                        .into_iter()
                        .map(|(n, t)| (n.unwrap_or("_".into()), t))
                        .collect();
                    let ret = Box::new(self.binary(5)?);
                    Expr::Pi(binders, ret, self.effects()?)
                } else if let [(None, _)] = entries.as_slice() {
                    entries.pop().unwrap().1
                } else {
                    return self.err("関数型なら ) の後に -> が必要");
                }
            }
            Tok::Punct("fn") => {
                self.expect("(")?;
                let mut params = Vec::new();
                while !self.eat(")") {
                    let n = self.ident()?;
                    let t = if self.eat(":") {
                        Some(self.ty()?)
                    } else {
                        None
                    };
                    params.push((n, t));
                    if !self.eat(",") {
                        self.expect(")")?;
                        break;
                    }
                }
                let ret = if self.eat("->") {
                    Some(Box::new(self.ret_ty()?))
                } else {
                    None
                };
                let effs = self.effects()?;
                self.expect("{")?;
                Expr::Lam(params, ret, effs, self.block()?)
            }
            Tok::Punct("{") => Expr::Block(self.block()?),
            Tok::Punct("if") => self.if_rest()?,
            Tok::Punct("while") => {
                let cond = self.expr()?;
                self.expect("{")?;
                Expr::While(Box::new(cond), self.block()?)
            }
            Tok::Punct("for") => {
                let x = self.ident()?;
                self.expect("in")?;
                let it = self.expr()?;
                let end = if self.eat("..") {
                    Some(Box::new(self.expr()?))
                } else {
                    None
                };
                self.expect("{")?;
                Expr::For(x, Box::new(it), end, self.block()?)
            }
            Tok::Punct("match") => {
                let s = self.expr()?;
                self.expect("{")?;
                let mut arms = Vec::new();
                while !self.eat("}") {
                    let ctor = self.ident()?;
                    let mut binds = Vec::new();
                    if self.eat("(") {
                        while !self.eat(")") {
                            binds.push(self.ident()?);
                            if !self.eat(",") {
                                self.expect(")")?;
                                break;
                            }
                        }
                    }
                    self.expect("=>")?;
                    let body = self.expr()?;
                    arms.push(Arm { ctor, binds, body });
                    self.eat(",");
                }
                Expr::Match(Box::new(s), arms)
            }
            Tok::Punct("handle") => {
                let body = self.expr()?;
                self.expect("{")?;
                let mut arms = Vec::new();
                while !self.eat("}") {
                    let ctor = self.ident()?;
                    self.expect("(")?;
                    let mut binds = Vec::new();
                    while !self.eat(")") {
                        binds.push(self.ident()?);
                        if !self.eat(",") {
                            self.expect(")")?;
                            break;
                        }
                    }
                    self.expect("=>")?;
                    let body = self.expr()?;
                    arms.push(Arm { ctor, binds, body });
                    self.eat(",");
                }
                Expr::Handle(Box::new(body), arms)
            }
            Tok::Punct("quote") => {
                self.expect("{")?;
                Expr::Quote(self.quote_body("}")?)
            }
            Tok::Punct("return") => {
                if self.at_newline() || matches!(self.peek(), Tok::Punct(";" | "}")) {
                    Expr::Return(None)
                } else {
                    Expr::Return(Some(Box::new(self.expr()?)))
                }
            }
            t => {
                if t != Tok::Eof {
                    self.pos -= 1;
                }
                return self.err("式が必要");
            }
        };
        while !self.at_newline() {
            if self.eat(".") {
                let name = self.ident()?;
                e = if !self.at_newline() && self.eat("(") {
                    Expr::Method(Box::new(e), name, self.args()?)
                } else {
                    Expr::Field(Box::new(e), name)
                };
            } else if self.eat("(") {
                e = Expr::Apply(Box::new(e), self.args()?);
            } else if self.eat("[") {
                let at = |p: &Self, k: usize| p.toks.get(p.pos + k).map(|t| t.tok.clone());
                e = if let (Some(Tok::Ident(x)), Some(Tok::Punct("->"))) =
                    (at(self, 0), at(self, 1))
                {
                    self.pos += 2;
                    Expr::Refine(Box::new(e), x, Box::new(self.expr()?))
                } else {
                    Expr::Index(Box::new(e), Box::new(self.expr()?))
                };
                self.expect("]")?;
            } else {
                break;
            }
        }
        Ok(e)
    }

    // 閉じ括弧 close までのトークンを読む。開き括弧は読み終えた状態で呼ぶ
    fn quote_body(&mut self, close: &str) -> R<Vec<Q>> {
        let open = if close == "}" { "{" } else { "(" };
        let mut out = Vec::new();
        let mut depth = 0;
        loop {
            let t = self.toks[self.pos].clone();
            match &t.tok {
                Tok::Eof => return self.err(&format!("quote の {close:?} が足りない")),
                Tok::Punct(p) if *p == close && depth == 0 => {
                    self.pos += 1;
                    return Ok(out);
                }
                Tok::Punct(p) if *p == close => depth -= 1,
                Tok::Punct(p) if *p == open => depth += 1,
                Tok::Punct("$") => {
                    self.pos += 1;
                    out.push(self.quote_dollar(t)?);
                    continue;
                }
                _ => {}
            }
            out.push(Q::Tok(t));
            self.pos += 1;
        }
    }

    // $x.y、${ 式 }、$( ... ) 区切り? *
    fn quote_dollar(&mut self, dollar: Token) -> R<Q> {
        if self.eat("{") {
            let e = self.expr()?;
            self.expect("}")?;
            return Ok(Q::Splice(dollar, e));
        }
        if self.eat("(") {
            let inner = self.quote_body(")")?;
            let sep = if self.peek() == &Tok::Punct("*") {
                None
            } else {
                Some(self.toks[self.pos].clone())
            };
            if sep.is_some() {
                self.pos += 1;
            }
            self.expect("*")?;
            return Ok(Q::Rep(inner, sep));
        }
        let (mut path, mut rest) = (vec![self.ident()?], Vec::new());
        while self.peek() == &Tok::Punct(".") && !self.toks[self.pos].gap_before {
            let Some(Tok::Ident(s)) = self.toks.get(self.pos + 1).map(|t| &t.tok) else {
                break;
            };
            path.push(s.clone());
            rest.extend(self.toks[self.pos..self.pos + 2].iter().cloned());
            self.pos += 2;
        }
        Ok(Q::Var { dollar, path, rest })
    }

    // "if" は読み終えた状態で呼ぶ
    fn if_rest(&mut self) -> R<Expr> {
        let cond = self.expr()?;
        self.expect("{")?;
        let then = self.block()?;
        let els = if !self.eat("else") {
            None
        } else if self.eat("if") {
            Some(Box::new(self.if_rest()?))
        } else {
            self.expect("{")?;
            Some(Box::new(Expr::Block(self.block()?)))
        };
        Ok(Expr::If(Box::new(cond), then, els))
    }
}
