pub use super::name::{Name, NameKind};
pub use super::modules::{Module, ModuleSet, load_modules};
use super::modules::{identify_tokens, source_tokens, text_source_tokens};
use crate::lexer::{Span, TPart, Tok, Token, lex};
use crate::meta::{Grammar, Node, Q};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// 構文木の節点の識別子。移動と複製では保持し、新しい節点には別の値を割り当てる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u64);

impl NodeId {
    // 実行時の名前を作るための番号
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl Default for NodeId {
    fn default() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Num(f64),
    Bool(bool),
    Str(String),
    Var(Name),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Call(Name, Vec<Expr>),
    // 式.名前 と 式.名前(引数)
    Field(Box<Expr>, Name),
    Method(Box<Expr>, Name, Vec<Expr>),
    Assign(Name, Box<Expr>),
    If(Box<Expr>, Block, Option<Box<Expr>>),
    While(Box<Expr>, Block),
    Break,
    Continue,
    Block(Block),
    Return(Option<Box<Expr>>),
    // 展開コードの無いマクロの照合結果
    Macro(String, Box<Node>),
    // quote { ... }。評価すると syn 型の値になる
    Quote(Vec<Q>),
    Array(Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    // 細分化型 型[変数 -> 条件]
    Refine(Box<Expr>, Name, Box<Expr>),
    Match(Box<Expr>, Vec<Arm>),
    // 無名関数 fn(引数) -> 戻り値の型 with 要求 { 本体 }。引数の型は期待する型から分かれば省ける
    Lam(
        Vec<(Name, Option<Expr>)>,
        Option<Box<Expr>>,
        // None は with の省略、Some([]) は with {}
        Option<Vec<Expr>>,
        Block,
    ),
    // 関数型 (x: A, ...) -> B with 要求。名前の無い引数は "_"。要求は効果の型の式（トレイトの要求・Async・効果の変数）
    Pi(Vec<(Name, Expr)>, Box<Expr>, Vec<Expr>),
    // 式の値の関数を呼ぶ
    Apply(Box<Expr>, Vec<Expr>),
    // 式.await。Async<T> の値を待つ。型検査が呼び出しか Apply に書き換えるので、出力には残らない
    Await(Box<Expr>),
    // with T(値) { 本体 } を書き換えた形。腕の ctor は操作（T::メソッド）の名前、binds は引数で、腕の値は #resume(値)。
    // 識別子は exit の行き先
    Handle(NodeId, Box<Expr>, Vec<Arm>),
    // exit 式。with の見出しの無名関数から、その with を式の値で抜ける。行き先（Handle の識別子）は型検査が書き込む
    Exit(Option<NodeId>, Box<Expr>),
    // for x in 式 { } と for x in a..b { }
    For(Name, Box<Expr>, Option<Box<Expr>>, Block),
    // ソース上の位置。エラー表示に使う
    At(Span, Box<Expr>),
    // 型検査後も式と一緒に移動する識別子。
    Id(NodeId, Box<Expr>),
    // 式?。Result の Err と Option の None で関数から戻る。型検査が let と match に書き換えるので、出力には残らない
    Try(Box<Expr>),
    // マクロの quote に書いた式。中の名前の公開範囲は、そのマクロを定義したモジュールの権限で判定する
    InModule(String, Box<Expr>),
}

impl Expr {
    // 位置の包みを外す
    // x = push(x, e) の形なら e。値を写さずにその場で足せる
    pub fn self_push(&self, x: &str) -> Option<&Expr> {
        match self.peel() {
            Expr::Call(n, a)
                if n == "push" && a.len() == 2 && matches!(a[0].peel(), Expr::Var(v) if v == x) =>
            {
                Some(&a[1])
            }
            _ => None,
        }
    }

    // x = set_at(x, i, e) の形なら (i, e)。値を写さずにその場で書き換えられる
    pub fn self_set(&self, x: &str) -> Option<(&Expr, &Expr)> {
        match self.peel() {
            Expr::Call(n, a)
                if n == "set_at"
                    && a.len() == 3
                    && matches!(a[0].peel(), Expr::Var(v) if v == x) =>
            {
                Some((&a[1], &a[2]))
            }
            _ => None,
        }
    }

    pub fn peel(&self) -> &Expr {
        match self {
            Expr::At(_, e) | Expr::Id(_, e) | Expr::InModule(_, e) => e.peel(),
            e => e,
        }
    }

    // 位置の包みを外す（可変）
    pub fn peel_mut(&mut self) -> &mut Expr {
        match self {
            Expr::At(_, e) | Expr::Id(_, e) | Expr::InModule(_, e) => e.peel_mut(),
            e => e,
        }
    }

    pub(crate) fn identified(&mut self) -> (NodeId, &mut Expr) {
        if !matches!(self, Expr::Id(..)) {
            let expression = std::mem::replace(self, Expr::Int(0));
            *self = Expr::Id(NodeId::default(), Box::new(expression));
        }
        let Expr::Id(id, expression) = self else { unreachable!() };
        (*id, expression)
    }
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: Name,
        mutable: bool,
        ty: Option<Expr>,
        init: Expr,
    },
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct Block {
    pub id: NodeId,
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}

#[derive(Debug, Clone)]
pub struct Fn {
    pub public: bool,
    // 空の with {} も、省略と区別して推論を止める
    pub effects_explicit: bool,
    // 宣言の位置（fn 名 まで）。利用者が書いた関数だけ Some。展開で作った関数は呼び出し位置
    pub span: Option<Span>,
    pub name: Name,
    pub params: Vec<(Name, Expr)>,
    // None は unit
    pub ret: Option<Expr>,
    pub body: Block,
    // type Name(引数) = 式 で書いた別名。引数が無ければ () を省いて呼べる
    pub alias: bool,
    // 定義したモジュール（利用者のファイルは ""）
    pub module: String,
    // params の先頭の何個がジェネリック引数か
    pub implicit: usize,
    // extern で宣言した JavaScript の関数。Some(モジュール)、グローバルの関数は Some("")
    pub js: Option<String>,
    // ! の後ろに書いた効果（Fail<E> などの型の式）
    pub effects: Vec<Expr>,
    // トレイトの要求の操作（T::m）なら、その要求の型の名前（Name::requirement）
    pub op: Option<Name>,
    // params の末尾の何個がトレイトの実装（<A: Show> の制約）か。呼び出しで省くと型検査が実装を探して足す
    pub insts: usize,
}

pub struct Parser<'s> {
    pub helpers: Rc<Program>,
    pub toks: Vec<Token>,
    pub pos: usize,
    pub src: &'s str,
    pub g: Rc<Grammar>,
    // マクロ展開の入れ子の深さ
    pub depth: usize,
    // 宣言された型の名前。直後の < を型引数として読む
    pub types: Rc<HashSet<Name>>,
    // ジェネリック引数を持つ関数の名前。f<int>(x) の < を型引数として読む
    pub generic_fns: Rc<HashSet<Name>>,
    // 大域の名前（関数・型・構築子・組み込み・パターンのリテラル）
    pub globals: Rc<HashSet<Name>>,
    // 構造体の名前から構築子とフィールド名の並び
    pub structs: Rc<HashMap<Name, Structure>>,
    // 戻り値の型を読んでいる間は、続く { を構造体の値と読まない
    pub no_struct: bool,
    // このファイルを含むモジュール（利用者のファイルは ""）
    pub module: String,
    // モジュールごとの名前（修飾しない形）
    pub mod_names: Rc<HashMap<String, HashSet<Name>>>,
    // モジュールごとの use
    pub uses: Rc<HashMap<String, Vec<Use>>>,
    // モジュールごとに、修飾して使えるモジュールの名前（-> を付けない use で取り込んだもの）。web.ok の web
    pub modules: Rc<HashMap<String, HashSet<String>>>,
    // 修飾した形の大域の名前（関数・型・構築子）
    pub known: Rc<HashSet<Name>>,
    // トレイトの名前（修飾した形）から、その構造体のフィールドの並び
    pub traits: Rc<HashMap<Name, Trait>>,
    // 今読んでいる式の名前を判定するモジュール。None は self.module
    pub perm: Option<String>,
    // フィールドの無い構築子（修飾した形）。パターンの中の名前を、構築子か束縛かに分ける
    pub nullary: Rc<HashSet<Name>>,
}

// trait 宣言。トレイトは構造体の型 Name<A> = { super#親: 親<A>, メソッド: 関数型 } にする
#[derive(Debug, Clone, Default)]
pub struct Trait {
    pub supers: Vec<Name>,
    // (メソッド名, 引数名, 既定の実装があるか)
    pub methods: Vec<(Name, Vec<Name>, bool)>,
    // 要求の操作（Name::m）を持つメソッドの名前
    pub ops: Vec<Name>,
}

use crate::error::Error;
pub use crate::error::R;

// data 宣言。
#[derive(Debug, Clone)]
pub struct Data {
    pub public: bool,
    // 宣言の位置（キーワードから名前まで）。型検査の誤りに行番号を付けるのに使う
    pub span: Option<Span>,
    pub name: Name,
    pub module: String,
    pub params: Vec<(Name, Expr)>,
    pub ctors: Vec<Ctor>,
}

#[derive(Debug, Clone)]
pub struct Ctor {
    pub name: Name,
    pub fields: Vec<(Name, Expr)>,
}

#[derive(Debug, Clone)]
pub struct Structure {
    pub constructor: Name,
    pub fields: Vec<Name>,
}

// 型の形に値の引数を使う依存型は受け付けない。細分化型の条件は値を参照できる。
fn depends_on_value(ty: &Expr, names: &[Name]) -> bool {
    match ty.peel() {
        Expr::Var(n) => names.contains(n),
        Expr::Call(n, xs) => names.contains(n) || xs.iter().any(|x| depends_on_value(x, names)),
        Expr::Unary(_, x) | Expr::Field(x, _) => depends_on_value(x, names),
        Expr::Binary(_, a, b) | Expr::Index(a, b) => depends_on_value(a, names) || depends_on_value(b, names),
        Expr::Refine(base, _, _) => depends_on_value(base, names),
        Expr::Pi(params, ret, effects) => {
            let mut local = names.to_vec();
            for (name, arg_ty) in params {
                if depends_on_value(arg_ty, &local) { return true; }
                if name != "_" { local.push(name.clone()); }
            }
            depends_on_value(ret, &local) || effects.iter().any(|x| depends_on_value(x, &local))
        }
        Expr::Array(xs) => xs.iter().any(|x| depends_on_value(x, names)),
        Expr::Apply(f, xs) => depends_on_value(f, names) || xs.iter().any(|x| depends_on_value(x, names)),
        _ => false,
    }
}

// match の腕。ctor が "_" なら残りすべて
#[derive(Debug, Clone)]
pub struct Arm {
    // 腕のパターン（構築子と束縛名）の位置。=> の手前まで
    pub span: Option<Span>,
    pub ctor: Name,
    pub binds: Vec<Name>,
    pub body: Expr,
}

enum TypeDecl {
    Data(Data),
    Alias(Fn),
}

#[derive(Debug, Default, Clone)]
pub struct Program {
    pub modules: Vec<Module>,
    pub fns: Vec<Fn>,
    pub datas: Vec<Data>,
    // test "名前" { 本体 } の (名前, 本体の関数名)。hibana test で実行する
    pub tests: Vec<(String, Name)>,
    // 型検査が記録した非同期処理の位置を、検査済みの構文木とともに渡す
    #[doc(hidden)]
    pub async_info: crate::typeck::Info,
}

// 組み込みのライブラリ。実行ファイルに埋め込むので、どこからでも use できる
pub(super) const LIBS: &[(&str, &str)] = &[
    ("std", include_str!("../../lib/std.hb")),
    ("std::math", include_str!("../../lib/math.hb")),
    ("std::wasm_web", include_str!("../../lib/wasm_web.hb")),
];

pub(super) const PRELUDE_NAMES: &[&str] = &["Result", "Ok", "Err", "parse_int", "refine", "spawn", "compile_wasm_file"];

// use した名前の取り込み方
#[derive(Debug, Clone)]
pub enum UseNames {
    // use web: web.ok のように修飾して使う
    Qualified,
    // use web -> *: すべて修飾なしで使える
    All,
    // use web -> {ok, Request}: 挙げた名前だけ修飾なしで使える
    List(Vec<Name>),
}

impl UseNames {
    // 名前 n を修飾なしで取り込むか。型 T を挙げれば、その型に属する関数も取り込む。
    pub fn takes(&self, n: &Name) -> bool {
        match self {
            UseNames::Qualified => false,
            UseNames::All => true,
            UseNames::List(ns) => {
                ns.iter().any(|name| name == n || matches!(&n.local().kind, NameKind::Associated(owner, _) if name == owner.local()))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Use {
    pub span: Option<Span>,
    pub module: String,
    pub names: UseNames,
}

// モジュール名 m の中の名前 n の内部での名前。利用者のファイル（m が空）では n のまま
pub fn qual(m: &str, n: impl Into<Name>) -> Name { Name::qualified(m, n.into()) }

// use に書いた名前から、モジュール名を作る。ファイルは拡張子を除いた名前（./js/util.ts は util）、
// node:path は path、@scope/pkg は pkg。名前に使えない文字は _ にし、キーワード（syntax など）には _ を足す
pub fn module_name(spec: &str) -> String {
    let last = spec.rsplit(['/', ':']).next().unwrap_or(spec);
    let stem = last.split('.').next().unwrap_or(last);
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if crate::lexer::KEYWORDS.contains(&name.as_str()) { name + "_" } else { name }
}

// use に書いたファイルの経路の起点（use を書いた .hb のディレクトリ）
thread_local! {
    static BASE: std::cell::RefCell<std::path::PathBuf> = std::cell::RefCell::new(".".into());
}

pub fn set_base_dir(p: &std::path::Path) {
    BASE.with(|b| *b.borrow_mut() = p.to_path_buf());
}

pub(crate) fn base_dir() -> std::path::PathBuf {
    BASE.with(|b| b.borrow().clone())
}

pub fn parse(src: &str) -> R<Program> {
    parse_modules(ModuleSet::from(src))
}

pub fn parse_modules(loaded: ModuleSet) -> R<Program> {
    let original = loaded
        .modules
        .iter()
        .map(|module| source_tokens(&module.source))
        .collect::<R<_>>()
        .and_then(|tokens| parse_modules_tokens(&loaded, tokens, 0));
    original.or_else(|error| {
        // 正常に読める文字列を優先する。読めないときだけ Text のアポストロフィを試す。
        let mut recovered = false;
        let tokens = loaded
            .modules
            .iter()
            .map(|module| {
                if let Some(tokens) = text_source_tokens(&module.source) {
                    recovered = true;
                    Ok(tokens)
                } else {
                    source_tokens(&module.source)
                }
            })
            .collect::<R<_>>();
        if recovered
            && let Ok(tokens) = tokens
            && let Ok(program) = parse_modules_tokens(&loaded, tokens, 0)
        {
            return Ok(program);
        }
        Err(error.with_sources(loaded.modules.iter().map(|module| &module.source)))
    })
}

#[derive(Default)]
struct Declarations {
    fn_starts: Vec<usize>,
    data_starts: Vec<usize>,
    extern_starts: Vec<usize>,
    trait_starts: Vec<usize>,
    test_starts: Vec<usize>,
    impl_starts: Vec<usize>,
    declarations: Vec<(usize, usize, usize, Node)>,
}

// ${ の後ろ s から、対応する } の位置（バイト数）を返す。{ } のネストと、中の文字列リテラルは飛ばす
// ponytail: s はエスケープを解いた後の文字列なので、式の中の文字列に \ や \' を書くと区切りを誤る。字句の段階で ${ } を読めば直る（Template と同じ方法）
fn interp_end(s: &str) -> Option<usize> {
    let (mut depth, mut quote) = (0, None);
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '{' => depth += 1,
                '}' if depth == 0 => return Some(i),
                '}' => depth -= 1,
                _ => {}
            },
        }
    }
    None
}

fn parse_modules_tokens(
    loaded: &ModuleSet,
    tokens: Vec<Vec<Token>>,
    expansion: usize,
) -> R<Program> {
    if expansion >= 64 {
        return Err("宣言マクロの展開が深すぎる".into());
    }
    let mut parsers: Vec<_> = loaded
        .modules
        .iter()
        .zip(tokens)
        .map(|(module, toks)| Parser {
            helpers: Rc::default(),
            toks,
            pos: 0,
            src: &module.source.text,
            g: Rc::default(),
            depth: 0,
            types: Rc::default(),
            generic_fns: Rc::default(),
            globals: Rc::default(),
            structs: Rc::default(),
            no_struct: false,
            module: module.name.clone(),
            mod_names: Rc::default(),
            uses: Rc::default(),
            modules: Rc::default(),
            known: Rc::default(),
            traits: Rc::default(),
            perm: None,
            nullary: Rc::default(),
        })
        .collect();
    let mut g = Grammar::new(&loaded.modules);
    let mut uses: HashMap<String, Vec<Use>> = HashMap::new();
    let mut modules: HashMap<String, HashSet<String>> = HashMap::new();
    let mut ends = Vec::new();
    for p in &mut parsers {
        // use・syntax・macro を先に読み、宣言の順序にかかわらず規則とマクロを参照できるようにする。
        let mut decl_end: HashMap<usize, usize> = HashMap::new();
        {
            let (mut depth, mut i) = (0, 0);
            while i < p.toks.len() {
                match &p.toks[i].tok {
                    Tok::Punct("(" | "[" | "{") => depth += 1,
                    Tok::Punct(")" | "]" | "}") => depth -= 1,
                    Tok::Punct(k @ ("use" | "syntax" | "macro")) if depth == 0 => {
                        p.pos = i;
                        match *k {
                            "use" => {
                                let u = p.use_decl()?;
                                if matches!(u.names, UseNames::Qualified) {
                                    modules.entry(p.module.clone()).or_default().insert(u.module.clone());
                                }
                                uses.entry(p.module.clone()).or_default().push(u);
                            }
                            "syntax" => p.syntax_decl(&mut g)?,
                            _ => p.macro_decl(&mut g)?,
                        }
                        decl_end.insert(i, p.pos);
                        i = p.pos;
                        continue;
                    }
                    _ => {}
                }
                i += 1;
            }
            p.pos = 0;
        }

        ends.push(decl_end);
    }
    g.validate(&uses)?;
    let grammar = Rc::new(g);
    for module in &loaded.modules {
        if module.name != "std" {
            uses.entry(module.name.clone()).or_default().push(Use {
                span: None,
                module: "std".into(),
                names: UseNames::List(PRELUDE_NAMES.iter().map(|name| Name::from(*name)).collect()),
            });
        }
    }
    let uses = Rc::new(uses);
    let modules = Rc::new(modules);
    let mut types: HashSet<Name> = ["Vec", "Ref", "Type", "Async", "Task"].map(Name::from).into();
    let mut generic_fns: HashSet<Name> = [Name::from("refine")].into();
    // 組み込みと利用者のファイルの名前・モジュール名・パターンのリテラル。マクロの衛生性で名前を付け替えない
    let mut globals: HashSet<Name> = [
        "js",
        "put",
        "input",
        "refine",
        "len",
        "at",
        "is_null",
        "int_of",
        "str_of",
        "error",
        "int",
        "number",
        "num_floor",
        "num_sqrt",
        "bool",
        "String",
        "syn",
        "unit",
        "ident",
        "to_str",
        "split",
        "starts_with",
        "slice",
        "chars",
        "code",
        "char_from_codepoint",
        "now_ms",
        "panic",
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
        "compile_file",
        "compile_wasm_file",
        "write_bytes",
        "Vec",
        "Type",
        "with",
    ]
    .map(Name::from)
    .into();
    // モジュールごとの名前（修飾しない形）と、修飾した形の大域の名前
    let mut mod_names: HashMap<String, HashSet<Name>> = HashMap::new();
    let mut known: HashSet<Name> = HashSet::new();
    // 名前を宣言する。同じモジュールで既に宣言していれば false
    let mut declare = |m: &str, n: &Name, globals: &mut HashSet<Name>| {
        mod_names
            .entry(m.to_string())
            .or_default()
            .insert(n.clone());
        if m.is_empty() {
            globals.insert(n.clone());
        }
        known.insert(qual(m, n))
    };
    let dup = |p: &Parser, i: usize, n: &str| {
        Err(Error::from(format!("名前 {n} が重複している")).at(p.toks[i].span))
    };

    let mut plans = Vec::new();
    for (p, decl_end) in parsers.iter_mut().zip(ends) {
        p.g = grammar.clone();
        p.uses = uses.clone();
        p.modules = modules.clone();
        let mut fn_starts = Vec::new();
        let mut data_starts = Vec::new();
        let mut extern_starts = Vec::new();
        let mut trait_starts = Vec::new();
        let mut test_starts = Vec::new();
        let mut impl_starts = Vec::new();
        let mut declarations = Vec::new();
        // 宣言の位置で照合した宣言のマクロ (始まり, 終わり, マクロの番号, 照合結果)
        // 1周目は宣言の名前を集める
        loop {
            let cur = p.module.clone();
            if p.eat("pub") && !matches!(p.peek(), Tok::Punct("fn" | "type" | "struct" | "enum" | "trait")) {
                return p.err("pub の後には fn・type・struct・enum・trait を書く");
            }
            match p.peek() {
                Tok::Eof => break,
                Tok::Punct("use" | "syntax" | "macro") => p.pos = decl_end[&p.pos],
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
                Tok::Punct("fn") => {
                    let k = 1;
                    if let Some(Tok::Ident(n)) = p.toks.get(p.pos + k).map(|t| t.tok.clone()) {
                        if p.toks.get(p.pos + k + 1).map(|t| &t.tok) == Some(&Tok::Punct("<")) {
                            generic_fns.insert(qual(&cur, &n));
                        }
                        if !declare(&cur, &n, &mut globals) {
                            return dup(&p, p.pos + k, &n);
                        }
                    }
                    fn_starts.push(p.pos);
                    p.skip_fn()?;
                }
                Tok::Punct(kw @ ("type" | "struct" | "enum")) => {
                    let kw = *kw;
                    if let Some(Tok::Ident(n)) = p.toks.get(p.pos + 1).map(|t| t.tok.clone()) {
                        types.insert(qual(&cur, &n));
                        if !declare(&cur, &n, &mut globals) {
                            return dup(&p, p.pos + 1, &n);
                        }
                    }
                    data_starts.push(p.pos);
                    let start = p.pos;
                    p.skip_decl();
                    if kw == "enum" {
                        for n in enum_ctors(&p.toks[start..p.pos]) {
                            declare(&cur, &n, &mut globals);
                        }
                    }
                }
                Tok::Punct("trait") => {
                    if let Some(Tok::Ident(n)) = p.toks.get(p.pos + 1).map(|t| t.tok.clone()) {
                        types.insert(qual(&cur, &n));
                        if !declare(&cur, &n, &mut globals) {
                            return dup(&p, p.pos + 1, &n);
                        }
                    }
                    trait_starts.push(p.pos);
                    p.skip_fn()?;
                }
                Tok::Punct("impl") => {
                    impl_starts.push(p.pos);
                    p.skip_fn()?;
                }
                // test は宣言の先頭でだけ特別な名前（変数名にも使える）
                Tok::Ident(t)
                    if t == "test"
                        && matches!(p.toks.get(p.pos + 1).map(|t| &t.tok), Some(Tok::Str(_))) =>
                {
                    test_starts.push(p.pos);
                    p.skip_fn()?;
                }
                _ => {
                    p.types = Rc::new(types.clone());
                    let start = p.pos;
                    if let Some((k, node)) = p.declaration_macro()? {
                        declarations.push((start, p.pos, k, node));
                    } else {
                        return p.err("fn・type・trait・impl・extern・syntax・macro・use・宣言マクロのいずれかが必要");
                    }
                }
            }
        }

        plans.push(Declarations {
            fn_starts,
            data_starts,
            extern_starts,
            trait_starts,
            test_starts,
            impl_starts,
            declarations,
        });
    }
    globals.extend(grammar.literals().into_iter().map(Name::from));
    globals.extend(PRELUDE_NAMES.iter().map(|name| Name::from(*name)));
    // 利用者のファイルで修飾して使うモジュールの名前は、マクロの衛生性で付け替えない
    globals.extend(modules.get("").into_iter().flatten().map(|name| Name::from(name.as_str())));
    let (types, generic_fns, globals, mod_names, known) = (
        Rc::new(types),
        Rc::new(generic_fns),
        Rc::new(globals),
        Rc::new(mod_names),
        Rc::new(known),
    );
    for p in &mut parsers {
        p.types = types.clone();
        p.generic_fns = generic_fns.clone();
        p.globals = globals.clone();
        p.mod_names = mod_names.clone();
        p.known = known.clone();
    }
    if plans.iter().any(|plan| !plan.declarations.is_empty()) {
        let mut tokens = Vec::new();
        for (p, plan) in parsers.iter_mut().zip(&plans) {
            let mut expanded = p.toks.clone();
            for (start, end, k, node) in plan.declarations.iter().rev() {
                p.pos = *end;
                let generated = p.expand_declaration(*k, node.clone(), p.toks[*start].span)?;
                expanded.splice(*start..*end, generated);
            }
            tokens.push(expanded);
        }
        return parse_modules_tokens(loaded, tokens, expansion + 1);
    }
    let mut prog = Program::default();
    for (p, plan) in parsers.iter_mut().zip(&plans) {
        for &s in &plan.data_starts {
            p.pos = s;
            match p.type_decl()? {
                TypeDecl::Data(d) => prog.datas.push(d),
                TypeDecl::Alias(f) => prog.fns.push(f),
            }
        }
    }
    let mut traits = HashMap::new();
    for (p, plan) in parsers.iter_mut().zip(&plans) {
        for &s in &plan.trait_starts {
            p.pos = s;
            let (ds, fs, t) = p.trait_decl()?;
            traits.insert(ds[0].name.clone(), t);
            p.traits = Rc::new(traits.clone());
            prog.datas.extend(ds);
            prog.fns.extend(fs);
        }
    }
    let structs: Rc<HashMap<Name, Structure>> = Rc::new(
        prog.datas
            .iter()
            .filter(|d| d.ctors.len() == 1 && d.ctors[0].name.structure_owner().is_some())
            .map(|d| {
                (
                    d.name.clone(),
                    Structure { constructor: d.ctors[0].name.clone(), fields: d.ctors[0].fields.iter().map(|field| field.0.clone()).collect() },
                )
            })
            .collect(),
    );
    let nullary: Rc<HashSet<Name>> = Rc::new(
        prog.datas.iter()
            .flat_map(|d| d.ctors.iter().filter(|c| c.fields.is_empty()).flat_map(move |c| [c.name.clone(), Name::associated(d.name.clone(), c.name.local().clone())]))
            .collect(),
    );
    let mut deferred = Vec::new();
    let mut implementation = 0;
    for (unit, (p, plan)) in parsers.iter_mut().zip(&plans).enumerate() {
        p.structs = structs.clone();
        p.nullary = nullary.clone();
        p.traits = Rc::new(traits.clone());
        for &start in &plan.fn_starts {
            p.pos = start;
            match p.func() {
                Ok(function) => prog.fns.push(function),
                Err(error) => deferred.push((unit, start, error)),
            }
        }
        for &start in &plan.extern_starts {
            p.pos = start;
            prog.fns.extend(p.extern_block()?);
        }
        for &start in &plan.impl_starts {
            p.pos = start;
            prog.fns.extend(p.impl_decl(implementation)?);
            implementation += 1;
        }
    }
    loop {
        let mut helpers = prog.clone();
        crate::resolve::resolve(&mut helpers, &uses, &known, &grammar)?;
        let helpers = Rc::new(helpers);
        for p in &mut parsers {
            p.helpers = helpers.clone();
        }
        if deferred.is_empty() {
            break;
        }
        let before = deferred.len();
        let mut failed = Vec::new();
        for (unit, start, _) in deferred {
            let p = &mut parsers[unit];
            p.pos = start;
            match p.func() {
                Ok(function) => prog.fns.push(function),
                Err(error) => failed.push((unit, start, error)),
            }
        }
        if failed.len() == before {
            return Err(failed.remove(0).2);
        }
        deferred = failed;
    }
    let mut cases = Vec::new();
    for (p, plan) in parsers.iter_mut().zip(&plans) {
        p.check_macros()?;
        let mut unit_cases = Vec::new();
        for &s in &plan.test_starts {
            p.pos = s + 1;
            let Tok::Str(label) = p.next() else {
                unreachable!()
            };
            p.expect("{")?;
            let before = unit_cases.len();
            let module = p.cur_module();
            let setup = p.test_cases(&label, Vec::new(), &module, &mut unit_cases)?;
            if unit_cases.len() == before {
                unit_cases.push((
                    label,
                    s,
                    module,
                    Block {
                        id: Default::default(),
                        stmts: setup,
                        tail: None,
                    },
                ));
            }
        }

        cases.extend(unit_cases.into_iter().map(|(label, start, module, body)| {
            (
                label,
                p.toks[start].site.unwrap_or(p.toks[start].span),
                module,
                body,
            )
        }));
    }
    for (k, (label, span, module, body)) in cases.into_iter().enumerate() {
        let name = qual(&module, Name::temporary("test", k));
        prog.tests.push((label, name.clone()));
        prog.fns.push(Fn {
            public: false,
            effects_explicit: false,
            span: Some(span),
            name,
            params: vec![],
            ret: None,
            body,
            alias: false,
            module,

            implicit: 0,
            js: None,
            effects: vec![],
            op: None,
            insts: 0,
        });
    }
    crate::resolve::resolve(&mut prog, &uses, &known, &grammar)?;
    prog.modules = loaded
        .modules
        .iter()
        .cloned()
        .map(|mut module| {
            module.uses = uses.get(&module.name).cloned().unwrap_or_default();
            module
        })
        .collect();
    Ok(prog)
}

// enum の宣言のトークン列から、構築子の名前。{ の中の深さ 1 で、{ か , か行頭の直後の名前（-> の戻り値の型の中は除く）
fn enum_ctors(toks: &[Token]) -> Vec<Name> {
    let (mut depth, mut start, mut in_ret) = (0, false, false);
    let mut out = Vec::new();
    for t in toks {
        if depth == 1 && (t.nl_before || matches!(t.tok, Tok::Punct("}"))) {
            in_ret = false;
            start = true;
        }
        match &t.tok {
            Tok::Punct("(" | "[" | "{") => {
                depth += 1;
                start = depth == 1;
                continue;
            }
            Tok::Punct(")" | "]" | "}") => depth -= 1,
            Tok::Punct(",") if depth == 1 => {
                in_ret = false;
                start = true;
                continue;
            }
            Tok::Punct("->") if depth == 1 => in_ret = true,
            Tok::Ident(n) if depth == 1 && start && !in_ret => out.push(n.clone()),
            _ => {}
        }
        start = false;
    }
    out
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
    pub(super) fn at_newline(&self) -> bool {
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
    pub fn ident(&mut self) -> R<Name> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.pos += 1;
                Ok(s)
            }
            _ => self.err("識別子が必要"),
        }
    }
    // m が、今のモジュールから修飾して使えるモジュールの名前か
    fn uses_module(&self, m: &str) -> bool {
        self.modules.get(&self.module).is_some_and(|ms| ms.contains(m))
    }

    // 今のトークンが名前なら、修飾した名前に書き換える。web.ok は1個の名前にし、型名は今のモジュールから見た名前にする
    // （構造体の値と <> の判定に使う。それ以外の名前は resolve で局所変数と区別してから解決する）
    pub(super) fn qualify_here(&mut self) {
        let Tok::Ident(n) = self.peek().clone() else {
            return;
        };
        if let NameKind::Associated(module, member) = &n.kind
            && self.uses_module(module.as_str())
        {
            self.toks[self.pos].tok = Tok::Ident(Name::qualified(module, *member.clone()));
            return;
        }
        let at = |k: usize| self.toks.get(self.pos + k).map(|t| t.tok.clone());
        if self.uses_module(n.as_str())
            && at(1) == Some(Tok::Punct("."))
            && !self.toks[self.pos + 1].gap_before
        {
            if let Some(Tok::Ident(x)) = at(2) {
                self.pos += 2;
                // 巻き戻して読み直した時は、既に書き換えてある
                if !matches!(x.kind, NameKind::Qualified(..)) {
                    self.toks[self.pos].tok = Tok::Ident(Name::qualified(&n, x));
                }
            }
            return;
        }
        let q = self.qualify(&n);
        if q != n
            && (self.types.contains(&q)
                || self.structs.contains_key(&q)
                // 関数の名前は f<型>(...) の時だけ。他は局所変数かもしれないので resolve に任せる
                || (self.generic_fns.contains(&q) && self.explicit_type_call(self.pos + 1)))
        {
            self.toks[self.pos].tok = Tok::Ident(q);
        }
    }

    // f<T>(...) と、局所変数と比較演算子の並びを区別する。
    fn explicit_type_call(&self, start: usize) -> bool {
        if self.toks.get(start).map(|t| &t.tok) != Some(&Tok::Punct("<")) { return false; }
        let (mut angles, mut nested) = (0, 0);
        for i in start..self.toks.len() {
            match self.toks[i].tok {
                Tok::Punct("(" | "[") => nested += 1,
                Tok::Punct(")" | "]") => { if nested == 0 { return false; } nested -= 1; },
                Tok::Punct("<") if nested == 0 => angles += 1,
                Tok::Punct(">") if nested == 0 => {
                    angles -= 1;
                    if angles == 0 { return self.toks.get(i + 1).map(|t| &t.tok) == Some(&Tok::Punct("(")); }
                }
                Tok::Punct("{" | "}" | ";") | Tok::Eof if nested == 0 => return false,
                _ => {}
            }
        }
        false
    }

    // `...{式}...` の式を、今の名前の環境で読む
    fn sub_expr(&self, toks: Vec<Token>) -> R<Expr> {
        let mut p = Parser {
            helpers: self.helpers.clone(),
            toks,
            pos: 0,
            src: self.src,
            g: self.g.clone(),
            depth: self.depth,
            types: self.types.clone(),
            generic_fns: self.generic_fns.clone(),
            globals: self.globals.clone(),
            structs: self.structs.clone(),
            no_struct: false,
            module: self.cur_module(),
            mod_names: self.mod_names.clone(),
            uses: self.uses.clone(),
            modules: self.modules.clone(),
            known: self.known.clone(),
            traits: self.traits.clone(),
            perm: None,
            nullary: self.nullary.clone(),
        };
        if p.peek() == &Tok::Eof {
            return p.err("文字列の中の { } に式が無い");
        }
        let e = p.expr()?;
        if p.peek() != &Tok::Eof {
            return p.err("文字列の中の { } には式を1つ書く");
        }
        Ok(e)
    }

    // 今のトークンを含むモジュール
    pub fn cur_module(&self) -> String {
        self.module.clone()
    }

    // 修飾していない名前 n を、今のモジュールから見た大域の名前にする。自分のモジュール、利用者のファイル、
    // use で取り込んだ名前の順に探す。見つからなければ n のまま
    pub fn qualify(&self, n: &Name) -> Name {
        if matches!(n.kind, NameKind::Qualified(..)) {
            return n.clone();
        }
        let m = self.cur_module();
        let mut cands = vec![qual(&m, n)];
        for u in self.uses.get(&m).into_iter().flatten() {
            if u.names.takes(n) { cands.push(qual(&u.module, n)); }
        }
        let known = |c: &Name| self.known.contains(c) || self.types.contains(c);
        cands.into_iter().find(known).unwrap_or_else(|| n.clone())
    }

    // use 名前、use 名前 -> *、use 名前 -> {a, b}、use "file.ts"
    fn use_decl(&mut self) -> R<Use> {
        let start = self.pos;
        self.expect("use")?;
        let module = match self.next() {
            Tok::Ident(n) => n.rsplit("::").next().unwrap().to_string(),
            Tok::Str(f) => super::modules::external_module_name(&f)
                .map_err(|error| error.at(self.toks[self.pos - 1].span))?,
            _ => return self.err("use の後にはライブラリ名かファイル名が必要"),
        };
        if self.toks[self.pos].tok == Tok::Punct(".") && !self.at_newline() {
            return self.err("use 名前 -> * または use 名前 -> { ... } で書く");
        }
        let names = if self.peek() == &Tok::Punct("->") {
            // use 'file' -> * と、use 'file' -> { a } -> { b }（改行して続けてよい）
            let mut ns = Vec::new();
            let mut all = false;
            while self.eat("->") {
                if self.eat("*") {
                    all = true;
                    continue;
                }
                self.expect("{")?;
                while !self.eat("}") {
                    ns.push(self.ident()?);
                    self.eat(",");
                }
            }
            if all { UseNames::All } else { UseNames::List(ns) }
        } else {
            UseNames::Qualified
        };
        Ok(Use { span: Some(self.span_of(start)), module, names })
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
        loop {
            while !self.eat("{") {
                if self.next() == Tok::Eof {
                    return self.err("関数の本体が無い");
                }
            }
            // 引数や戻り値の関数型に書いた with {} も、本体と区別する
            if self.pos >= 2 && matches!(&self.toks[self.pos - 2].tok, Tok::Ident(n) if n == "with") {
                self.expect("}")?;
            } else {
                break;
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
        let start = self.pos;
        let public = start > 0 && self.toks[start - 1].tok == Tok::Punct("pub");
        self.expect("fn")?;
        let module = self.cur_module();
        let name = qual(&module, &self.ident()?);
        // 行番号の表示に使う。fn 名 までを指す
        let span = self.span_of(start);
        // ジェネリック引数は引数の先頭に置き、呼び出しで省けるようにする
        let mut params = if self.eat("<") {
            self.type_params()?
        } else {
            vec![]
        };
        // 制約 <A: Show> の実装は引数の末尾に置く
        let insts: Vec<_> = params
            .extract_if(.., |p| p.0.is_instance())
            .collect();
        let implicit = params.len();
        self.expect("(")?;
        params.extend(self.params()?);
        let ret = if self.eat("->") {
            let ret = self.ret_ty()?;
            let value_names: Vec<_> = params[implicit..].iter().map(|(n, _)| n.clone()).collect();
            if depends_on_value(&ret, &value_names) {
                return self.err("戻り値の型に引数の値は使えない");
            }
            Some(ret)
        } else {
            None
        };
        let effects_explicit = matches!(self.peek(), Tok::Ident(n) if n == "with");
        if public && !effects_explicit {
            return Err(Error::from(format!("公開関数 {name} には with を明示する（効果がなければ with {{}}）")).at(span));
        }
        let (effects, named) = self.requirements(true)?;
        self.expect("{")?;
        let n_insts = insts.len();
        params.extend(insts);
        let mut body = self.block()?;
        // 名前を付けた要求（with Logger(logger)）は、本体の先頭で名前を束縛する。型検査が要求の型を付け、logger.log(m) を要求の呼び出しにする
        for (local, eff) in named.into_iter().rev() {
            body.stmts.insert(0, Stmt::Let { name: local, mutable: false, ty: None, init: Expr::Call(Name::from("#named"), vec![eff]) });
        }
        Ok(Fn {
            public,
            effects_explicit,
            span: Some(span),
            name,
            params,
            ret,
            body,
            alias: false,
            module,

            implicit,
            js: None,
            effects,
            op: None,
            insts: n_insts,
        })
    }

    // 関数型の戻り値の後ろの with 要求
    fn pi_rest(&mut self, binders: Vec<(Name, Expr)>, ret: Box<Expr>) -> R<Expr> {
        let (effects, _) = self.requirements(false)?;
        Ok(Expr::Pi(binders, ret, effects))
    }

    // with 要求, ... 。トレイト T の要求は効果の型 T<引数>（Name::requirement）、それ以外（Async・効果の変数）はそのまま。
    // named なら、トレイトの要求に with Logger(logger) と名前を付けられる。引数の並びの中の関数型では、, の後が 名前: なら次の引数
    // ponytail: 名前の無い引数の並び (int, (int) -> int with Log, String) では String も要求と読むので、関数型を括弧で囲む
    fn requirements(&mut self, named: bool) -> R<(Vec<Expr>, Vec<(Name, Expr)>)> {
        if self.peek() == &Tok::Punct("!") {
            return self.err("要求は ! でなく with で書く（with Fail<E>, Async）");
        }
        let (mut effects, mut names) = (Vec::new(), Vec::new());
        if !matches!(self.peek(), Tok::Ident(n) if n == "with") {
            return Ok((effects, names));
        }
        self.pos += 1;
        if self.eat("{") {
            self.expect("}")?;
            return Ok((effects, names));
        }
        loop {
            self.qualify_here();
            let name = self.ident()?;
            let mut args = Vec::new();
            if self.eat("<") {
                loop {
                    // 閉じの > を比較と読まないよう、加減乗除より弱い演算子は読まない
                    args.push(self.binary(5)?);
                    if !self.eat(",") {
                        self.expect(">")?;
                        break;
                    }
                }
            }
            let tr = self.trait_name(&name);
            let head = tr.clone().map_or(name.clone(), Name::requirement);
            if effects.iter().any(|e: &Expr| matches!(e.peel(), Expr::Var(n) | Expr::Call(n, _) if *n == head)) {
                return self.err_at(format!("要求 {name} は一度だけ書ける"));
            }
            let e = if args.is_empty() { Expr::Var(head) } else { Expr::Call(head, args) };
            if self.peek() == &Tok::Punct("(") && !self.at_newline() {
                if tr.is_none() {
                    return self.err_at(format!("{name} はトレイトでない（名前を付けられるのはトレイトの要求だけ）"));
                }
                if !named {
                    return self.err("関数型の要求には名前を付けない");
                }
                self.pos += 1;
                let local = self.ident()?;
                self.expect(")")?;
                names.push((local, e.clone()));
            }
            effects.push(e);
            let next_param = matches!(self.toks.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Ident(_)))
                && self.toks.get(self.pos + 2).map(|t| &t.tok) == Some(&Tok::Punct(":"));
            if next_param || !self.eat(",") {
                break;
            }
        }
        Ok((effects, names))
    }

    // 名前がトレイトなら、修飾した形
    fn trait_name(&self, c: &Name) -> Option<Name> {
        if self.traits.contains_key(c) {
            return Some(c.clone());
        }
        let q = self.qualify(c);
        self.traits.contains_key(&q).then_some(q)
    }

    // trait Name<E>: 親 + ... { fn メソッド(self, 引数) -> 型 with 要求 [{ 既定の実装 }] ... }。対象の型は型引数 Self
    // 構造体の型 Name<Self, E> と、メソッドを呼ぶ関数 Name#m（実装を末尾の引数 inst#0 で受け取る）・Name#get・既定の実装の関数にする。
    // with Name<E> で要求した時のために、効果の型 Name<E>（Name::requirement）と、self を除いた引数の操作 Name::m も作る
    // ponytail: メソッドは自分のジェネリック引数を持てない。持たせるならメソッドの関数型を依存関数型にする
    fn trait_decl(&mut self) -> R<(Vec<Data>, Vec<Fn>, Trait)> {
        let start = self.pos;
        let public = start > 0 && self.toks[start - 1].tok == Tok::Punct("pub");
        self.expect("trait")?;
        let module = self.cur_module();
        let name = qual(&module, &self.ident()?);
        let span = self.span_of(start);
        // trait Name<E> { fn f(self) } は、対象の型を先頭の型引数 Self に持つ trait Name<Self, E>
        let mut params = vec![(Name::from("Self"), Expr::Var("Type".into()))];
        if self.eat("<") {
            let extra = self.type_params()?;
            if extra.iter().any(|p| p.0.is_instance()) {
                return self.err("トレイトの型引数には制約を書けない");
            }
            params.extend(extra);
        }
        let me = Expr::Call(
            name.clone(),
            params.iter().map(|p| Expr::Var(p.0.clone())).collect(),
        );
        let mut fields = Vec::new();
        let mut info = Trait::default();
        if self.eat(":") {
            loop {
                let mut s = self.binary(6)?;
                // trait Ord: Eq の親は Eq<Self>
                if let Expr::Var(n) = s.peel() {
                    s = Expr::Call(n.clone(), vec![Expr::Var(params[0].0.clone())]);
                }
                let Expr::Call(sn, _) = s.peel() else {
                    return self.err("親のトレイトは 名前 か 名前<引数> で書く");
                };
                let sn = self.qualify(sn);
                fields.push((Name::supertrait(sn.clone()), s.clone()));
                info.supers.push(sn);
                if !self.eat("+") {
                    break;
                }
            }
        }
        self.expect("{")?;
        let mut fns = Vec::new();
        let with_inst = |ps: &[(Name, Expr)]| {
            let mut v = params.clone();
            v.extend(ps.iter().cloned());
            v.push((Name::instance(0), me.clone()));
            v
        };
        while !self.eat("}") {
            let mstart = self.pos;
            self.expect("fn")?;
            let m = self.ident()?;
            let mspan = self.span_of(mstart);
            self.expect("(")?;
            let ps = self.params()?;
            let ret = if self.eat("->") {
                self.ret_ty()?
            } else {
                Expr::Var("unit".into())
            };
            let effects_explicit = matches!(self.peek(), Tok::Ident(n) if n == "with");
            if public && !effects_explicit {
                return Err(Error::from(format!("公開トレイトのメソッド {m} には with を明示する（効果がなければ with {{}}）")).at(mspan));
            }
            let (effects, _) = self.requirements(false)?;
            let vars: Vec<Expr> = ps.iter().map(|p| Expr::Var(p.0.clone())).collect();
            let field = Expr::Field(Box::new(Expr::Var(Name::instance(0))), m.clone());
            let mk = |name: Name, body: Block| Fn {
                public,
                effects_explicit,
                span: Some(mspan),
                name,
                params: with_inst(&ps),
                ret: Some(ret.clone()),
                body,
                alias: false,
                module: module.clone(),

                implicit: params.len(),
                js: None,
                effects: effects.clone(),
                op: None,
                insts: 1,
            };
            fns.push(mk(
                Name::trait_method(name.clone(), m.clone()),
                Block { id: Default::default(),
                    stmts: vec![],
                    // with Async のメソッドは、実装の関数の Async<T> を待って返す
                    tail: Some(Box::new(if effects.iter().any(|e| matches!(e.peel(), Expr::Var(n) if n == "Async")) {
                        Expr::Await(Box::new(Expr::Apply(Box::new(field), vars)))
                    } else {
                        Expr::Apply(Box::new(field), vars)
                    })),
                },
            ));
            let has = self.eat("{");
            if has {
                let b = self.block()?;
                fns.push(mk(Name::default_method(name.clone(), m.clone()), b));
            }
            // 要求の操作。Self を使うメソッド（fn eq(self, other: Self)）は、対象の型が決まらないので操作にしない
            let self_type = [Name::from("Self")];
            if ps.first().is_some_and(|p| p.0 == "self")
                && !ps[1..].iter().any(|p| depends_on_value(&p.1, &self_type))
                && !depends_on_value(&ret, &self_type)
            {
                let mut ops_params = params[1..].to_vec();
                ops_params.extend(ps[1..].iter().cloned());
                fns.push(Fn {
                    public,
                    effects_explicit: true,
                    span: Some(mspan),
                    name: Name::associated(name.clone(), m.clone()),
                    params: ops_params,
                    ret: Some(ret.clone()),
                    body: Block { id: Default::default(), stmts: vec![], tail: None },
                    alias: false,
                    module: module.clone(),
                    implicit: params.len() - 1,
                    js: None,
                    effects: effects.clone(),
                    op: Some(Name::requirement(name.clone())),
                    insts: 0,
                });
                info.ops.push(m.clone());
            }
            fields.push((m.clone(), Expr::Pi(ps.clone(), Box::new(ret), effects)));
            info.methods
                .push((m, ps.into_iter().map(|p| p.0).collect(), has));
            self.eat(",");
        }
        fns.push(Fn {
            public,
            effects_explicit: true,
            span: Some(span),
            name: Name::trait_accessor(name.clone()),
            params: with_inst(&[]),
            ret: Some(me.clone()),
            body: Block { id: Default::default(),
                stmts: vec![],
                tail: Some(Box::new(Expr::Var(Name::instance(0)))),
            },
            alias: false,
            module: module.clone(),

            implicit: params.len(),
            js: None,
            effects: vec![],
            op: None,
            insts: 1,
        });
        let ctor = Ctor {
            name: Name::structure(name.clone()),
            fields,
        };
        let requirement = Data { public, span: Some(span), name: Name::requirement(name.clone()), module: module.clone(), params: params[1..].to_vec(), ctors: vec![] };
        Ok((
            vec![
                Data {
                    public,
                    span: Some(span),
                    name,
                    module,
                    params,
                    ctors: vec![ctor],
                },
                requirement,
            ],
            fns,
            info,
        ))
    }

    // impl<G: 制約> Trait<型> { fn メソッド(引数) -> 型 { 本体 } ... }。
    // 実装の値を作る関数 Trait#implK（制約の実装を引数に取る）と、メソッドの関数 Trait#implK#メソッド にする
    fn impl_decl(&mut self, k: usize) -> R<Vec<Fn>> {
        let start = self.pos;
        self.expect("impl")?;
        let module = self.cur_module();
        let mut generics = if self.eat("<") {
            self.type_params()?
        } else {
            vec![]
        };
        let cons: Vec<_> = generics
            .extract_if(.., |p| p.0.is_instance())
            .collect();
        // impl Name<E> for 型 は impl Name<型, E>
        let tname = self.ret_ty()?;
        let (tn, extra) = match tname.peel() {
            Expr::Var(tn) => (tn.clone(), vec![]),
            Expr::Call(tn, xs) => (tn.clone(), xs.clone()),
            _ => return self.err("impl は impl トレイト for 型 { ... } と書く"),
        };
        self.expect("for")?;
        let mut targs = vec![self.ret_ty()?];
        targs.extend(extra);
        let head = Expr::Call(tn.clone(), targs);
        let Expr::Call(tn, targs) = head.peel() else {
            unreachable!()
        };
        // 型を省いた self の型（Self）は、対象の型にする
        let me = targs.first().cloned();
        let Some(q) = self.trait_name(tn) else {
            return self.err_at(format!("{tn} はトレイトでない"));
        };
        let tr = self.traits[&q].clone();
        let base = Name::qualified(&module, Name::implementation(q.local().clone(), k));
        self.expect("{")?;
        let span = self.span_of(start);
        let mut fns = Vec::new();
        let mut given = HashMap::new();
        while !self.eat("}") {
            let mut f = self.func()?;
            if let Some(me) = &me {
                for (_, t) in &mut f.params {
                    if matches!(t.peel(), Expr::Var(n) if n == "Self") {
                        *t = me.clone();
                    }
                }
            }
            let m = f.name.local().clone();
            if !tr.methods.iter().any(|x| x.0 == m) {
                return self.err_at(format!("トレイト {q} にメソッド {m} は無い"));
            }
            f.name = Name::implementation_method(base.clone(), m.clone());
            let mut ps = generics.clone();
            ps.append(&mut f.params);
            ps.extend(cons.iter().cloned());
            f.params = ps;
            f.implicit = generics.len();
            f.insts = cons.len();
            given.insert(m, f.name.clone());
            fns.push(f);
        }
        let mut args: Vec<Expr> = tr
            .supers
            .iter()
            .map(|s| Expr::Call(Name::trait_accessor(s.clone()), vec![]))
            .collect();
        for (m, ps, has) in &tr.methods {
            let target = match given.get(m) {
                Some(n) => n.clone(),
                None if *has => Name::default_method(q.clone(), m.clone()),
                None => return self.err_at(format!("{q} のメソッド {m} の実装が無い")),
            };
            let call = Expr::Call(target, ps.iter().map(|p| Expr::Var(p.clone())).collect());
            let body = Block { id: Default::default(),
                stmts: vec![],
                tail: Some(Box::new(call)),
            };
            args.push(Expr::Lam(
                ps.iter().map(|p| (p.clone(), None)).collect(),
                None,
                None,
                body,
            ));
        }
        let mut params = generics.clone();
        params.extend(cons.iter().cloned());
        fns.push(Fn {
            public: false,
            effects_explicit: true,
            span: Some(span),
            name: base,
            params,
            ret: Some(head),
            body: Block { id: Default::default(),
                stmts: vec![],
                tail: Some(Box::new(Expr::Call(Name::structure(q.clone()), args))),
            },
            alias: false,
            module,

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
            let public = self.eat("pub");
            let start = self.pos;
            self.expect("fn")?;
            let module = self.cur_module();
            let name = qual(&module, &self.ident()?);
            let span = self.span_of(start);
            self.expect("(")?;
            let params = self.params()?;
            let ret = if self.eat("->") {
                Some(self.ret_ty()?)
            } else {
                None
            };
            let effects_explicit = matches!(self.peek(), Tok::Ident(n) if n == "with");
            if public && !effects_explicit {
                return Err(Error::from(format!("公開関数 {name} には with を明示する（効果がなければ with {{}}）")).at(span));
            }
            let (effects, _) = self.requirements(false)?;
            let body = Block { id: Default::default(),
                stmts: vec![],
                tail: None,
            };
            fns.push(Fn {
                public,
                effects_explicit,
                span: Some(span),
                name,
                params,
                ret,
                body,
                alias: false,
                module,

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
    fn params(&mut self) -> R<Vec<(Name, Expr)>> {
        let mut params = Vec::new();
        while !self.eat(")") {
            let n = self.ident()?;
            // 型を省いた self は Self 型（トレイトの対象の型。impl では対象の型に置き換える）
            let ty = if n == "self" && self.peek() != &Tok::Punct(":") {
                Expr::Var("Self".into())
            } else {
                self.expect(":")?;
                self.ty()?
            };
            let value_names: Vec<_> = params.iter().map(|(name, _): &(Name, Expr)| name.clone()).collect();
            if depends_on_value(&ty, &value_names) {
                return self.err("引数の型に前の引数の値は使えない");
            }
            if matches!(ty.peel(), Expr::Var(name) if name == "Type") {
                return self.err("型の値を関数や構築子の引数にできない");
            }
            params.push((n, ty));
            if !self.eat(",") {
                self.expect(")")?;
                break;
            }
        }
        Ok(params)
    }

    // type Name(引数) = | 構築子 | ... は新しい型、type Name(引数) = 式 は別名
    // <名前: 型, ...>。"<" は読み終えた状態で呼ぶ。閉じの > を比較と読まないよう、加減乗除より弱い演算子は読まない
    fn type_params(&mut self) -> R<Vec<(Name, Expr)>> {
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
            // <A: Fail<E>> の実装の型は Fail<A, E>
            let traits: Option<Vec<(Name, Vec<Expr>)>> = leaves
                .iter()
                .map(|l| match l.peel() {
                    Expr::Var(c) => self.trait_name(c).map(|c| (c, vec![])),
                    Expr::Call(c, xs) => self.trait_name(c).map(|c| (c, xs.clone())),
                    _ => None,
                })
                .collect();
            if let Some(ts) = traits {
                params.push((n.clone(), Expr::Var("Type".into())));
                for (c, xs) in ts {
                    let k = params.iter().filter(|p| p.0.is_instance()).count();
                    let mut args = vec![Expr::Var(n.clone())];
                    args.extend(xs);
                    params.push((Name::instance(k), Expr::Call(c, args)));
                }
            } else {
                if !matches!(t.peel(), Expr::Var(name) if name == "Type" || name == "Effect") {
                    return self.err("型引数には型、Effect、またはトレイトの制約が必要");
                }
                params.push((n, t));
            }
            if !self.eat(",") {
                self.expect(">")?;
                return Ok(params);
            }
        }
    }

    // struct Name<引数> { フィールド: 型, ... } と struct Name<引数>(型, ...) は構造体（構築子 Name{} を1つ持つ型）。
    // enum Name<引数> { 構築子(フィールド: 型, ...) -> 戻り値の型, ... } は直和型。
    // type Name<引数> = 式 は別名か型を計算する関数で、実行時には消える
    fn type_decl(&mut self) -> R<TypeDecl> {
        let start = self.pos;
        let public = start > 0 && self.toks[start - 1].tok == Tok::Punct("pub");
        let kw = self.next();
        let module = self.cur_module();
        let name = qual(&module, &self.ident()?);
        let span = self.span_of(start);
        let params = if self.eat("<") {
            self.type_params()?
        } else {
            vec![]
        };
        if kw == Tok::Punct("enum") {
            self.expect("{")?;
            let mut ctors = Vec::new();
            while !self.eat("}") {
                let c = self.ident()?;
                // 名前の無いフィールド（Circle(Point, int)）は 0, 1, ... と名付ける
                let mut fields = Vec::new();
                if self.eat("(") {
                    while !self.eat(")") {
                        let named = matches!(self.peek(), Tok::Ident(_))
                            && self.toks.get(self.pos + 1).is_some_and(|t| t.tok == Tok::Punct(":"));
                        let n = if named {
                            let n = self.ident()?;
                            self.expect(":")?;
                            n
                        } else {
                            fields.len().to_string().into()
                        };
                        let ty = self.ty()?;
                        if matches!(ty.peel(), Expr::Var(name) if name == "Type") {
                            return self.err("型の値を構築子の引数にできない");
                        }
                        fields.push((n, ty));
                        if !self.eat(",") {
                            self.expect(")")?;
                            break;
                        }
                    }
                }
                if self.eat("->") {
                    return self.err("構築子の戻り値の型は指定できない");
                }
                ctors.push(Ctor {
                    name: qual(&module, &c),
                    fields,
                });
                self.eat(",");
            }
            return Ok(TypeDecl::Data(Data {
                public,
                span: Some(span),
                name,
                module,
                params,
                ctors,
            }));
        }
        let tuple = kw == Tok::Punct("struct") && self.eat("(");
        if kw == Tok::Punct("struct") && !tuple {
            self.expect("{")?;
        }
        if kw == Tok::Punct("type") {
            self.expect("=")?;
            if self.peek() == &Tok::Punct("{") || self.has_bar() {
                return self.err("構造体は struct Name { ... }、直和型は enum Name { ... } で書く");
            }
        }
        if kw == Tok::Punct("struct") {
            let mut fields = Vec::new();
            // struct Name(型, ...) のフィールドは 0, 1, ... で、x.0 で取り出す
            while !self.eat(if tuple { ")" } else { "}" }) {
                if tuple {
                    let ty = self.ty()?;
                    if matches!(ty.peel(), Expr::Var(name) if name == "Type") {
                        return self.err("型の値をフィールドにできない");
                    }
                    fields.push((fields.len().to_string().into(), ty));
                } else {
                    let n = self.ident()?;
                    self.expect(":")?;
                    let ty = self.ty()?;
                    let value_names: Vec<_> = fields.iter().map(|(name, _): &(Name, Expr)| name.clone()).collect();
                    if depends_on_value(&ty, &value_names) {
                        return self.err("フィールドの型に前の値は使えない");
                    }
                    if matches!(ty.peel(), Expr::Var(name) if name == "Type") {
                        return self.err("型の値をフィールドにできない");
                    }
                    fields.push((n, ty));
                }
                self.eat(",");
            }
            let ctor = Ctor {
                name: if tuple { Name::tuple_structure(name.clone()) } else { Name::structure(name.clone()) },
                fields,
            };
            return Ok(TypeDecl::Data(Data {
                public,
                span: Some(span),
                name,
                module,
                params,
                ctors: vec![ctor],
            }));
        }
        {
            let e = self.ty()?;
            if matches!(e.peel(), Expr::If(..)) {
                return self.err("型別名には型を指定する");
            }
            let body = Block { id: Default::default(),
                stmts: vec![],
                tail: Some(Box::new(e)),
            };
            let ret = Some(Expr::Var("Type".into()));
            return Ok(TypeDecl::Alias(Fn {
                public,
                effects_explicit: true,
                span: Some(span),
                name,
                params,
                ret,
                body,
                alias: true,
                module,

                js: None,
                implicit: 0,
                effects: vec![],
                op: None,
                insts: 0,
            }));
        }
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
                    "fn" | "type" | "syntax" | "macro" | "use" | "module" | "extern"
                    | "trait" | "impl" | "struct" | "enum" | "pub",
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
                    "fn" | "type" | "syntax" | "macro" | "use" | "module" | "extern"
                    | "trait" | "impl" | "struct" | "enum" | "pub",
                ) if depth == 0 => {
                    return;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    // let 名前 = 式 と、構造体を分解する let パターン = 式（フィールドごとの let にする）
    fn let_stmt(&mut self) -> R<Vec<Stmt>> {
        self.expect("let")?;
        let mutable = self.eat("mut");
        if !mutable
            && matches!(self.peek(), Tok::Ident(_))
            && matches!(self.toks.get(self.pos + 1).map(|t| &t.tok), Some(Tok::Punct("{" | "(")))
            && !self.toks[self.pos + 1].nl_before
        {
            let start = self.pos;
            let pattern = self.value_pattern(true)?;
            self.expect("=")?;
            let init = self.expr()?;
            self.end_stmt()?;
            let t = Name::pattern("□", NodeId::default().raw() as usize);
            let mut stmts = vec![Stmt::Let { name: t.clone(), mutable: false, ty: None, init }];
            let end = self.pos;
            self.pos = start;
            self.destructure(pattern, t, &mut stmts)?;
            self.pos = end;
            return Ok(stmts);
        }
        let name = self.ident()?;
        let ty = if self.eat(":") {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect("=")?;
        let init = self.expr()?;
        self.end_stmt()?;
        Ok(vec![Stmt::Let {
            name,
            mutable,
            ty,
            init,
        }])
    }

    // マクロのパターンの Stmt 用
    pub fn stmt(&mut self) -> R<Stmt> {
        if let Some(e) = self.try_macros(true)? {
            self.eat(";");
            return Ok(Stmt::Expr(e));
        }
        if self.peek() == &Tok::Punct("let") {
            let mut stmts = self.let_stmt()?;
            if stmts.len() != 1 {
                return self.err("マクロの文には let の分解を書けない");
            }
            return Ok(stmts.remove(0));
        }
        let e = self.expr()?;
        self.eat(";");
        Ok(Stmt::Expr(e))
    }

    // "{" は読み終えた状態で呼ぶ
    // test と describe の本体（"{" は読み終えた状態で呼ぶ）。it '名前' { } ごとに、前に書いた文を前置きにした本体を cases に足す。
    // describe '名前' { } は名前をまとめるだけ。返り値は、この本体の文（it が無い test の本体になる）
    fn test_cases(
        &mut self,
        label: &str,
        mut setup: Vec<Stmt>,
        module: &str,
        cases: &mut Vec<(String, usize, String, Block)>,
    ) -> R<Vec<Stmt>> {
        let mut groups = 0;
        // 最後の it・describe より後にある最初の文の位置
        let mut trailing = None;
        while !self.eat("}") {
            let at = self.pos;
            if let Tok::Ident(w) = self.peek().clone()
                && (w == "describe" || w == "it")
                && let Some(Tok::Str(name)) = self.toks.get(at + 1).map(|t| t.tok.clone())
                && self.toks.get(at + 2).is_some_and(|t| t.tok == Tok::Punct("{"))
            {
                self.pos += 3;
                let label = format!("{label} / {name}");
                let before = cases.len();
                if w == "it" {
                    let body = self.block()?;
                    let mut stmts = setup.clone();
                    stmts.extend(body.stmts);
                    cases.push((label, at, module.to_string(), Block { id: Default::default(), stmts, tail: body.tail }));
                } else {
                    self.test_cases(&label, setup.clone(), module, cases)?;
                    if cases.len() == before {
                        self.pos = at;
                        return self.err_at(format!("describe '{name}' に it が無い"));
                    }
                }
                self.end_stmt()?;
                groups += 1;
                trailing = None;
                continue;
            }
            if groups > 0 && trailing.is_none() {
                trailing = Some(at);
            }
            if let Some(e) = self.try_macros(true)? {
                self.end_stmt()?;
                setup.push(Stmt::Expr(e));
            } else if self.peek() == &Tok::Punct("let") {
                setup.extend(self.let_stmt()?);
            } else {
                setup.push(Stmt::Expr(self.expr()?));
                if !self.eat(";") {
                    self.end_stmt()?;
                }
            }
        }
        if let Some(at) = trailing {
            self.pos = at;
            return self.err_at("最後の it・describe の後の文はどの it でも実行されない（前に置く）".into());
        }
        Ok(setup)
    }

    pub fn block(&mut self) -> R<Block> {
        let mut stmts = Vec::new();
        loop {
            if self.eat("}") {
                return Ok(Block { id: Default::default(), stmts, tail: None });
            }
            if let Some(e) = self.try_macros(true)? {
                self.end_stmt()?;
                stmts.push(Stmt::Expr(e));
                continue;
            }
            if self.peek() == &Tok::Punct("let") {
                stmts.extend(self.let_stmt()?);
                continue;
            }
            let e = self.expr()?;
            if self.eat(";") {
                stmts.push(Stmt::Expr(e));
            } else if self.eat("}") {
                return Ok(Block { id: Default::default(),
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

    pub(super) fn wrap(&self, start: usize, e: Expr) -> Expr {
        Expr::At(self.span_of(start), Box::new(e))
    }

    fn unary(&mut self) -> R<Expr> {
        for op in ["-", "!"] {
            if self.eat(op) {
                return Ok(Expr::Unary(op, Box::new(self.unary()?)));
            }
        }
        // ** は単項演算子より強く、右から結合する（-x ** 2 は -(x ** 2)、2 ** 3 ** 2 は 2 ** (3 ** 2)）
        let start = self.pos;
        let base = self.primary()?;
        if self.peek() == &Tok::Punct("**") && !self.at_newline() {
            self.pos += 1;
            let base = self.wrap(start, base);
            let exp = self.unary()?;
            return Ok(Expr::Binary("**", Box::new(base), Box::new(exp)));
        }
        Ok(base)
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
        // 名前で始まる式は、そのトークンを書いたモジュールの権限で判定する。権限が今と違えば InModule で包む
        let token = &self.toks[self.pos];
        if matches!(token.tok, Tok::Ident(_) | Tok::Template(..)) {
            let origin = token.origin.clone().unwrap_or_else(|| self.cur_module());
            if origin != self.perm.clone().unwrap_or_else(|| self.cur_module()) {
                let outer = self.perm.replace(origin.clone());
                let e = self.primary();
                self.perm = outer;
                return Ok(Expr::InModule(origin, Box::new(e?)));
            }
        }
        let origin = self.toks[self.pos].origin.clone();
        self.qualify_here();
        let mut e = match self.next() {
            Tok::Int(n) => Expr::Int(n),
            Tok::Float(n) => Expr::Num(n),
            Tok::Str(s) => Expr::Str(s),
            // `a{x}b` は 'a' + to_str(x) + 'b'
            Tok::Template(_, parts) => {
                let mut acc = Expr::Str(String::new());
                for part in parts {
                    let e = match part {
                        TPart::Lit(s) => Expr::Str(s),
                        TPart::Code(toks) => {
                            let toks = toks.into_iter().map(|t| Token { origin: origin.clone(), ..t }).collect();
                            Expr::Call("to_str".into(), vec![self.sub_expr(toks)?])
                        }
                    };
                    acc = Expr::Binary("+", Box::new(acc), Box::new(e));
                }
                acc
            }
            Tok::Punct("true") => Expr::Bool(true),
            Tok::Punct("false") => Expr::Bool(false),
            // with T(値), U(値) { 本体 } は、本体で起きた T・U の要求を値で処理する（Handle にする）。値はブロックに入る時に一度評価する。
            // 値が無名関数なら、その本体を腕にする（T のメソッドが 1 つの時だけ）。無名関数の値を返すと、呼んだ所が続く（#resume）。
            // それ以外の値は T の実装を持つ値で、メソッドごとに、その値のメソッドを呼んで続ける腕を作る
            // 腕は型検査の後に、隠れた引数（文脈）で呼び出し先へ渡す（check/context.rs）
            Tok::Ident(name) if name == "with" && matches!(self.peek(), Tok::Ident(_)) => {
                let id = NodeId::default();
                let mut stmts = Vec::new();
                let mut arms = Vec::new();
                loop {
                    let astart = self.pos;
                    self.qualify_here();
                    let trait_name = self.ident()?;
                    let Some(tr) = self.trait_name(&trait_name) else {
                        self.pos -= 1;
                        if trait_name == "Async" {
                            return self.err_at("Async は with で提供できない（Async を処理するのはランタイムと spawn だけ）".into());
                        }
                        return self.err_at(format!("{trait_name} はトレイトでない"));
                    };
                    let span = Some(self.span_of(astart));
                    self.expect("(")?;
                    let value = self.expr()?;
                    self.expect(")")?;
                    let info = self.traits[&tr].clone();
                    let ops: Vec<&(Name, Vec<Name>, bool)> = info.methods.iter().filter(|m| info.ops.contains(&m.0)).collect();
                    let mut lam = &value;
                    while let Expr::At(_, x) | Expr::Id(_, x) | Expr::InModule(_, x) = lam {
                        lam = x;
                    }
                    if let Expr::Lam(params, ret, effs, body) = lam {
                        let [(m, _, _)] = ops.as_slice() else {
                            self.pos = astart;
                            return self.err_at(format!(
                                "無名関数で提供できるのは、メソッドが 1 つのトレイトだけ（{trait_name} は {} 個）。impl を持つ値を渡す",
                                info.methods.len()
                            ));
                        };
                        if info.methods.len() != 1 {
                            self.pos = astart;
                            return self.err_at(format!("無名関数で提供できるのは、メソッドが 1 つのトレイトだけ（{trait_name} は {} 個）", info.methods.len()));
                        }
                        if effs.is_some() {
                            self.pos = astart;
                            return self.err_at("提供する無名関数には with を書かない（使える要求はトレイトのメソッドの宣言で決まる）".into());
                        }
                        // 書いた引数と戻り値の型は、let の型として検査する
                        let mut inner: Vec<Stmt> = params
                            .iter()
                            .filter_map(|(p, t)| t.clone().map(|t| Stmt::Let { name: p.clone(), mutable: false, ty: Some(t), init: Expr::Var(p.clone()) }))
                            .collect();
                        let mut v = Expr::Block(body.clone());
                        if let Some(rt) = ret {
                            let tmp = Name::temporary("ret", NodeId::default().raw() as usize);
                            inner.push(Stmt::Let { name: tmp.clone(), mutable: false, ty: Some((**rt).clone()), init: v });
                            v = Expr::Var(tmp);
                        }
                        let v = Expr::Block(Block { id: Default::default(), stmts: inner, tail: Some(Box::new(v)) });
                        arms.push(Arm {
                            span,
                            ctor: Name::associated(tr.clone(), m.clone()),
                            binds: params.iter().map(|p| p.0.clone()).collect(),
                            body: resume_block(v),
                        });
                    } else {
                        let tmp = Name::temporary("with", NodeId::default().raw() as usize);
                        stmts.push(Stmt::Let { name: tmp.clone(), mutable: false, ty: None, init: value });
                        for (m, ps, _) in ops {
                            let binds: Vec<Name> = ps[1..].to_vec();
                            let mut args = vec![Expr::Var(tmp.clone())];
                            args.extend(binds.iter().map(|b| Expr::Var(b.clone())));
                            let call = Expr::Call(Name::trait_method(tr.clone(), m.clone()), args);
                            arms.push(Arm { span, ctor: Name::associated(tr.clone(), m.clone()), binds, body: resume_block(call) });
                        }
                    }
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect("{")?;
                let body = self.block()?;
                let handle = Expr::Handle(id, Box::new(Expr::Block(body)), arms);
                Expr::Block(Block { id: Default::default(), stmts, tail: Some(Box::new(handle)) })
            }
            // exit 式。with の見出しの無名関数から、その with を抜ける
            Tok::Punct("exit") => Expr::Exit(None, Box::new(self.expr()?)),
            // struct Name(型, ...) の値 Name(式, ...)。構築子 Name{} の呼び出しにする
            Tok::Ident(name)
                if self
                    .structs
                    .get(&name)
                    .is_some_and(|structure| structure.constructor.structure_style() == Some(super::name::StructureStyle::Tuple))
                    && !self.at_newline()
                    && self.eat("(") =>
            {
                Expr::Call(self.structs[&name].constructor.clone(), self.args()?)
            }
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
                let structure = self.structs[&name].clone();
                let mut args = Vec::new();
                for f in &structure.fields {
                    let Some(i) = given.iter().position(|g| &g.0 == f) else {
                        return self.err_at(format!("構造体 {name} のフィールド {f} が無い"));
                    };
                    args.push(given.remove(i).1);
                }
                if let Some((f, _)) = given.first() {
                    return self.err_at(format!("構造体 {name} にフィールド {f} は無い"));
                }
                Expr::Call(structure.constructor, args)
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
                if self.generic_fns.contains(&name) && self.explicit_type_call(self.pos) =>
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
                    self.pi_rest(binders, ret)?
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
                let explicit = matches!(self.peek(), Tok::Ident(n) if n == "with");
                let (effs, _) = self.requirements(false)?;
                self.expect("{")?;
                Expr::Lam(params, ret, explicit.then_some(effs), self.block()?)
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
                    let astart = self.pos;
                    let pattern = self.value_pattern(true)?;
                    let span = Some(self.span_of(astart));
                    self.expect("->")?;
                    arms.push((span, pattern, self.expr()?));
                    self.eat(",");
                }
                self.compile_match(s, arms)?
            }
            Tok::Punct("quote") => {
                self.expect("{")?;
                Expr::Quote(self.quote_body("}")?)
            }
            Tok::Punct("break") => Expr::Break,
            Tok::Punct("continue") => Expr::Continue,
            Tok::Punct("return") => {
                if self.at_newline() || matches!(self.peek(), Tok::Punct(";" | "}")) {
                    Expr::Return(None)
                } else {
                    Expr::Return(Some(Box::new(self.expr()?)))
                }
            }
            // .do() を使うマクロの本体に書いた return（meta.rs の template_body）。展開後に式の値へ直す
            Tok::Punct("macro_return") => {
                let v = if self.at_newline() || matches!(self.peek(), Tok::Punct(";" | "}")) {
                    Expr::Block(Block { id: Default::default(), stmts: vec![], tail: None })
                } else {
                    self.expr()?
                };
                Expr::Call("#return".into(), vec![v])
            }
            t => {
                if t != Tok::Eof {
                    self.pos -= 1;
                }
                return self.err("式が必要");
            }
        };
        while !self.at_newline() {
            if self.eat("?") {
                e = Expr::Try(Box::new(e));
            } else if self.eat(".") {
                if self.eat("await") {
                    e = Expr::Await(Box::new(e));
                    continue;
                }
                // struct Name(型, ...) のフィールドは x.0
                let name = match self.peek().clone() {
                    Tok::Int(k) => {
                        self.pos += 1;
                        k.to_string().into()
                    }
                    _ => self.ident()?,
                };
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

    // quote の文字列リテラル s の ${式} を読む。断片と、式を文字列にした値を + でつないだ式にする
    fn quote_text(&self, s: &str) -> R<Expr> {
        let plus = |a: Expr, b: Expr| Expr::Binary("+", Box::new(a), Box::new(b));
        let mut acc = Expr::Str(String::new());
        let mut rest = s;
        while let Some(at) = rest.find("${") {
            let after = &rest[at + 2..];
            let Some(end) = interp_end(after) else {
                return self.err("quote の文字列の ${ に対応する } が無い");
            };
            let mut tokens = lex(&after[..end])?;
            identify_tokens(&mut tokens, self.toks[self.pos].span.source);
            let value = self.sub_expr(tokens)?;
            acc = plus(acc, Expr::Str(rest[..at].into()));
            acc = plus(acc, Expr::Call("to_str".into(), vec![value]));
            rest = &after[end + 1..];
        }
        Ok(plus(acc, Expr::Str(rest.into())))
    }

    // 閉じ括弧 close までのトークンを読む。開き括弧は読み終えた状態で呼ぶ
    fn quote_body(&mut self, close: &str) -> R<Vec<Q>> {
        let open = if close == "}" { "{" } else { "(" };
        let mut out = Vec::new();
        let mut depth = 0;
        // 開いている括弧の並び。{ は「直前が名前」なら構造体の波括弧として true にする
        let mut opens: Vec<bool> = Vec::new();
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
            // 文字列リテラルの中の ${式} は、断片と式の文字列化をつないだ式として差し込む（全体でも途中でも書ける）
            if let Tok::Str(s) = &t.tok
                && s.contains("${")
            {
                let value = self.quote_text(s)?;
                out.push(Q::Splice(t, value));
                self.pos += 1;
                continue;
            }
            // 構造体リテラル・宣言の フィールド名 Name { x: 式 } の x。直前が { か , で、直後が :
            // ponytail: $( ... ) の中は外の括弧が見えないので、その中の Name { x: ... } の x は改名される。外から見える括弧の並びを渡すようにすれば直る
            let field = matches!(t.tok, Tok::Ident(_))
                && opens.last() == Some(&true)
                && self
                    .toks
                    .get(self.pos + 1)
                    .is_some_and(|n| n.tok == Tok::Punct(":"))
                && matches!(out.last(), Some(Q::Tok(p)) if matches!(p.tok, Tok::Punct("{" | ",")));
            match &t.tok {
                Tok::Punct("(" | "[") => opens.push(false),
                Tok::Punct("{") => opens
                    .push(matches!(out.last(), Some(Q::Tok(p)) if matches!(p.tok, Tok::Ident(_)))),
                Tok::Punct(")" | "]" | "}") => {
                    opens.pop();
                }
                _ => {}
            }
            out.push(if field { Q::Field(t) } else { Q::Tok(t) });
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
        let (mut path, mut rest) = (vec![self.ident()?.to_string()], Vec::new());
        while self.peek() == &Tok::Punct(".") && !self.toks[self.pos].gap_before {
            let Some(Tok::Ident(s)) = self.toks.get(self.pos + 1).map(|t| &t.tok) else {
                break;
            };
            path.push(s.to_string());
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

// with の腕の本体 { #resume(値) }。ブロックの識別子は、JavaScript で待つ腕（async にする無名関数）の印に使う
fn resume_block(v: Expr) -> Expr {
    Expr::Block(Block { id: NodeId::default(), stmts: vec![], tail: Some(Box::new(Expr::Call("#resume".into(), vec![v]))) })
}
