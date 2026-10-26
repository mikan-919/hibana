use crate::lexer::{Tok, Token};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Type {
    Int,
    Bool,
    Unit,
    // return の型。どの型とも一致する
    Never,
}

#[derive(Debug)]
pub enum Expr {
    Int(i64),
    Bool(bool),
    Var(String),
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Assign(String, Box<Expr>),
    If(Box<Expr>, Block, Option<Box<Expr>>),
    While(Box<Expr>, Block),
    Block(Block),
    Return(Option<Box<Expr>>),
}

#[derive(Debug)]
pub enum Stmt {
    Let {
        name: String,
        mutable: bool,
        ty: Option<Type>,
        init: Expr,
    },
    Expr(Expr),
}

#[derive(Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}

#[derive(Debug)]
pub struct Fn {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub body: Block,
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

type R<T> = Result<T, String>;

pub fn parse(toks: Vec<Token>) -> R<Vec<Fn>> {
    let mut p = Parser { toks, pos: 0 };
    let mut fns = Vec::new();
    while p.peek() != &Tok::Eof {
        fns.push(p.func()?);
    }
    Ok(fns)
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

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if t != Tok::Eof {
            self.pos += 1;
        }
        t
    }
    fn err<T>(&self, msg: &str) -> R<T> {
        let t = &self.toks[self.pos];
        Err(format!("{}行: {msg}（実際は {:?}）", t.line, t.tok))
    }
    // 次のトークンが前のトークンと別の行にあるか
    fn at_newline(&self) -> bool {
        self.pos > 0 && self.toks[self.pos].line > self.toks[self.pos - 1].line
    }
    // 文の区切り: ";"、改行、"}" の直前のいずれか
    fn end_stmt(&mut self) -> R<()> {
        if self.eat(";") || self.at_newline() || self.peek() == &Tok::Punct("}") {
            Ok(())
        } else {
            self.err("文の区切り（\";\" か改行）が必要")
        }
    }
    fn eat(&mut self, p: &str) -> bool {
        if matches!(self.peek(), Tok::Punct(q) if *q == p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, p: &str) -> R<()> {
        if self.eat(p) {
            Ok(())
        } else {
            self.err(&format!("{p:?} が必要"))
        }
    }
    fn ident(&mut self) -> R<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.pos += 1;
                Ok(s)
            }
            _ => self.err("識別子が必要"),
        }
    }
    fn ty(&mut self) -> R<Type> {
        match self.ident()?.as_str() {
            "int" => Ok(Type::Int),
            "bool" => Ok(Type::Bool),
            _ => {
                self.pos -= 1;
                self.err("不明な型")
            }
        }
    }

    fn func(&mut self) -> R<Fn> {
        self.expect("fn")?;
        let name = self.ident()?;
        self.expect("(")?;
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
        let ret = if self.eat("->") {
            self.ty()?
        } else {
            Type::Unit
        };
        self.expect("{")?;
        Ok(Fn {
            name,
            params,
            ret,
            body: self.block()?,
        })
    }

    // "{" は読み終えた状態で呼ぶ
    fn block(&mut self) -> R<Block> {
        let mut stmts = Vec::new();
        loop {
            if self.eat("}") {
                return Ok(Block { stmts, tail: None });
            }
            if self.eat("let") {
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
                stmts.push(Stmt::Let {
                    name,
                    mutable,
                    ty,
                    init,
                });
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

    fn expr(&mut self) -> R<Expr> {
        let after = self.toks.get(self.pos + 1).map(|t| &t.tok);
        if let (Tok::Ident(name), Some(Tok::Punct("="))) = (self.peek().clone(), after) {
            self.pos += 2;
            return Ok(Expr::Assign(name, Box::new(self.expr()?)));
        }
        self.binary(0)
    }

    fn binary(&mut self, min: u8) -> R<Expr> {
        let mut lhs = self.unary()?;
        while let Some((prec, op)) = binop(self.peek()) {
            // 行頭の演算子は前の行に続けない（"-x" を新しい文として読むため）
            if prec < min || self.at_newline() {
                break;
            }
            self.pos += 1;
            let rhs = self.binary(prec + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> R<Expr> {
        for op in ["-", "!"] {
            if self.eat(op) {
                return Ok(Expr::Unary(op, Box::new(self.unary()?)));
            }
        }
        self.primary()
    }

    fn primary(&mut self) -> R<Expr> {
        match self.next() {
            Tok::Int(n) => Ok(Expr::Int(n)),
            Tok::Punct("true") => Ok(Expr::Bool(true)),
            Tok::Punct("false") => Ok(Expr::Bool(false)),
            Tok::Ident(name) => {
                if self.at_newline() || !self.eat("(") {
                    return Ok(Expr::Var(name));
                }
                let mut args = Vec::new();
                while !self.eat(")") {
                    args.push(self.expr()?);
                    if !self.eat(",") {
                        self.expect(")")?;
                        break;
                    }
                }
                Ok(Expr::Call(name, args))
            }
            Tok::Punct("(") => {
                let e = self.expr()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Punct("{") => Ok(Expr::Block(self.block()?)),
            Tok::Punct("if") => self.if_rest(),
            Tok::Punct("while") => {
                let cond = self.expr()?;
                self.expect("{")?;
                Ok(Expr::While(Box::new(cond), self.block()?))
            }
            Tok::Punct("return") => {
                if self.at_newline() || matches!(self.peek(), Tok::Punct(";" | "}")) {
                    Ok(Expr::Return(None))
                } else {
                    Ok(Expr::Return(Some(Box::new(self.expr()?))))
                }
            }
            _ => {
                self.pos -= 1;
                self.err("式が必要")
            }
        }
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
