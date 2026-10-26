use crate::lexer::{Span, Tok, Token, lex};
use crate::meta::{Grammar, Node, Q};
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
    // partial fn。停止性を検査しない代わりに、型の計算で展開しない
    pub partial: bool,
}

pub struct Parser<'s> {
    pub toks: Vec<Token>,
    pub pos: usize,
    pub src: &'s str,
    pub g: Rc<Grammar>,
    // マクロ展開の入れ子の深さ
    pub depth: usize,
}

use crate::error::Error;
pub use crate::error::R;

// data 宣言。構築子の ret は添字付きの型族の戻り値の型（無ければ Name(params)）
#[derive(Debug)]
pub struct Data {
    pub name: String,
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
}

pub fn parse(src: &str) -> R<Program> {
    let mut p = Parser {
        toks: lex(src)?,
        pos: 0,
        src,
        g: Rc::default(),
        depth: 0,
    };
    let mut g = Grammar::default();
    let mut fn_starts = Vec::new();
    let mut data_starts = Vec::new();
    // 1周目は syntax と macro だけを読む（宣言より前の関数でもマクロを使えるようにする）
    loop {
        match p.peek() {
            Tok::Eof => break,
            Tok::Punct("syntax") => p.syntax_decl(&mut g)?,
            Tok::Punct("macro") => p.macro_decl(&mut g)?,
            Tok::Punct("fn" | "partial") => {
                fn_starts.push(p.pos);
                p.skip_fn()?;
            }
            Tok::Punct("type") => {
                data_starts.push(p.pos);
                p.skip_decl();
            }
            _ => return p.err("fn・type・syntax・macro のいずれかが必要"),
        }
    }
    g.validate()?;
    p.g = Rc::new(g);
    p.check_macros()?;
    let mut prog = Program::default();
    for s in data_starts {
        p.pos = s;
        match p.type_decl()? {
            TypeDecl::Data(d) => prog.datas.push(d),
            TypeDecl::Alias(f) => prog.fns.push(f),
        }
    }
    for s in fn_starts {
        p.pos = s;
        prog.fns.push(p.func()?);
    }
    Ok(prog)
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
        let name = self.ident()?;
        self.expect("(")?;
        let params = self.params()?;
        let ret = if self.eat("->") {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect("{")?;
        Ok(Fn {
            name,
            params,
            ret,
            body: self.block()?,
            alias: false,
            partial,
        })
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
    fn type_decl(&mut self) -> R<TypeDecl> {
        self.expect("type")?;
        let name = self.ident()?;
        let params = if self.eat("(") {
            self.params()?
        } else {
            vec![]
        };
        self.expect("=")?;
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
                partial: false,
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
            ctors.push(Ctor { name, fields, ret });
            if !self.eat("|") {
                break;
            }
        }
        Ok(TypeDecl::Data(Data {
            name,
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
                Tok::Punct("fn" | "partial" | "type" | "syntax" | "macro") | Tok::Eof
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
                Tok::Punct("fn" | "partial" | "type" | "syntax" | "macro") if depth == 0 => return,
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
        let mut e = match self.next() {
            Tok::Int(n) => Expr::Int(n),
            Tok::Str(s) => Expr::Str(s),
            Tok::Punct("true") => Expr::Bool(true),
            Tok::Punct("false") => Expr::Bool(false),
            // 大文字で始まる名前に空白なしで続く < は型引数。比較の < と区別する
            Tok::Ident(name)
                if name.starts_with(|c: char| c.is_ascii_uppercase())
                    && self.peek() == &Tok::Punct("<")
                    && !self.toks[self.pos].gap_before =>
            {
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
            Tok::Ident(name) => {
                if !self.at_newline() && self.eat("(") {
                    Expr::Call(name, self.args()?)
                } else {
                    Expr::Var(name)
                }
            }
            Tok::Punct("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                e
            }
            Tok::Punct("{") => Expr::Block(self.block()?),
            Tok::Punct("if") => self.if_rest()?,
            Tok::Punct("while") => {
                let cond = self.expr()?;
                self.expect("{")?;
                Expr::While(Box::new(cond), self.block()?)
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
