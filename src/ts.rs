// TypeScript の型宣言（.ts と .d.ts）から、hibana のモジュールの本文を作る。
// export した関数・interface・type を読み、関数は extern の宣言に、interface とオブジェクト型は構造体にする。
// 本体は読み飛ばす。読めない宣言（ジェネリクス・可変長引数など）は無視する
// Promise<T> を返す関数は、T を返す Async の効果の関数にする
// ponytail: TypeScript の型の一部だけを読む。共用体・ジェネリクスなどは js 型か無視になる
use crate::error::R;

// TypeScript の型を hibana の型にしたもの
enum Ty {
    Int,
    Str,
    Bool,
    Unit,
    Js,
    Vec(Box<Ty>),
    Fn(Vec<Ty>, Box<Ty>),
    Obj(Vec<(String, Ty)>),
    Named(String),
    // Promise<T>。関数の戻り値なら T を返す Async の効果の関数にする
    Promise(Box<Ty>),
}

fn lex(src: &str) -> Vec<String> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && cs.get(i + 1) == Some(&'/') {
            while i < cs.len() && cs[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && cs.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < cs.len() && !(cs[i] == '*' && cs[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c == '"' || c == '\'' || c == '`' {
            // 文字列は中身を見ないので、まとめて1個のトークンにする
            let s = i;
            i += 1;
            while i < cs.len() && cs[i] != c {
                i += if cs[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            out.push(cs[s..i.min(cs.len())].iter().collect());
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let s = i;
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_' || cs[i] == '$') {
                i += 1;
            }
            out.push(cs[s..i].iter().collect());
        } else if c == '=' && cs.get(i + 1) == Some(&'>') {
            out.push("=>".into());
            i += 2;
        } else if c == '.' && cs.get(i + 1) == Some(&'.') && cs.get(i + 2) == Some(&'.') {
            out.push("...".into());
            i += 3;
        } else {
            out.push(c.to_string());
            i += 1;
        }
    }
    out
}

struct P {
    t: Vec<String>,
    i: usize,
    fns: Vec<String>,
    structs: Vec<String>,
    anon: usize,
    // このファイルで宣言した型の名前
    names: Vec<String>,
}

impl P {
    fn peek(&self) -> &str {
        self.t.get(self.i).map_or("", |s| s.as_str())
    }
    fn at(&self, k: usize) -> &str {
        self.t.get(self.i + k).map_or("", |s| s.as_str())
    }
    fn eat(&mut self, s: &str) -> bool {
        if self.peek() == s {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn next(&mut self) -> String {
        let s = self.peek().to_string();
        self.i += 1;
        s
    }

    // 対応する閉じ括弧まで読み飛ばす。今のトークンが開き括弧
    fn skip_group(&mut self) {
        let (open, close) = match self.peek() {
            "{" => ("{", "}"),
            "(" => ("(", ")"),
            "[" => ("[", "]"),
            "<" => ("<", ">"),
            _ => return,
        };
        let mut depth = 0;
        while self.i < self.t.len() {
            let s = self.next();
            if s == open {
                depth += 1;
            } else if s == close {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }

    // 宣言の終わり（; か改行の先の次の宣言）まで読み飛ばす
    fn skip_decl(&mut self) {
        while self.i < self.t.len() && !matches!(self.peek(), ";" | "export") {
            if matches!(self.peek(), "{" | "(" | "[") {
                self.skip_group();
            } else {
                self.i += 1;
            }
        }
        self.eat(";");
    }

    fn ty(&mut self) -> Option<Ty> {
        let mut t = self.primary()?;
        while self.peek() == "[" && self.at(1) == "]" {
            self.i += 2;
            t = Ty::Vec(Box::new(t));
        }
        // 共用体は hibana の型で表せないので js にする
        if self.peek() == "|" {
            while self.eat("|") {
                self.primary()?;
                while self.peek() == "[" && self.at(1) == "]" {
                    self.i += 2;
                }
            }
            return Some(Ty::Js);
        }
        Some(t)
    }

    fn primary(&mut self) -> Option<Ty> {
        match self.peek() {
            "(" => {
                // (a: T, ...) => R は関数型、それ以外は括弧で囲んだ型
                let is_fn =
                    self.at(1) == ")" || matches!(self.at(2), ":" | "?") || self.at(1) == "...";
                self.i += 1;
                if !is_fn {
                    let t = self.ty()?;
                    self.eat(")");
                    return Some(t);
                }
                let ps = self.params()?;
                if !self.eat("=>") {
                    return None;
                }
                let r = self.ty()?;
                Some(Ty::Fn(ps, Box::new(r)))
            }
            "{" => {
                self.i += 1;
                let mut fs = Vec::new();
                while !self.eat("}") {
                    let n = self.next();
                    if n.is_empty() {
                        return None;
                    }
                    // メソッドや添字の型は読めないので js のフィールドにする
                    if self.peek() == "(" || n == "[" {
                        self.skip_decl_in_obj();
                        continue;
                    }
                    let optional = self.eat("?");
                    self.eat(":");
                    let t = self.ty()?;
                    fs.push((n, if optional { Ty::Js } else { t }));
                    let _ = self.eat(";") || self.eat(",");
                }
                Some(Ty::Obj(fs))
            }
            s if s.starts_with(['"', '\'', '`']) || s.starts_with(|c: char| c.is_ascii_digit()) => {
                self.i += 1;
                Some(Ty::Js)
            }
            _ => {
                let n = self.next();
                Some(match n.as_str() {
                    "number" => Ty::Int,
                    "string" => Ty::Str,
                    "boolean" => Ty::Bool,
                    "void" | "undefined" => Ty::Unit,
                    "Promise" if self.peek() == "<" => {
                        self.i += 1;
                        let t = self.ty()?;
                        self.eat(">");
                        Ty::Promise(Box::new(t))
                    }
                    "Array" | "ReadonlyArray" if self.peek() == "<" => {
                        self.i += 1;
                        let t = self.ty()?;
                        self.eat(">");
                        Ty::Vec(Box::new(t))
                    }
                    "" => return None,
                    _ => {
                        if self.peek() == "<" {
                            self.skip_group();
                            Ty::Js
                        } else {
                            Ty::Named(n)
                        }
                    }
                })
            }
        }
    }

    // オブジェクト型の中の読めないメンバーを読み飛ばす
    fn skip_decl_in_obj(&mut self) {
        while self.i < self.t.len() && !matches!(self.peek(), ";" | "," | "}") {
            if matches!(self.peek(), "{" | "(" | "[" | "<") {
                self.skip_group();
            } else {
                self.i += 1;
            }
        }
        let _ = self.eat(";") || self.eat(",");
    }

    // (a: T, b?: U)。"(" は読み終えた状態で呼ぶ
    fn params(&mut self) -> Option<Vec<Ty>> {
        let mut ps = Vec::new();
        while !self.eat(")") {
            if self.peek() == "..." {
                return None;
            }
            self.next();
            let optional = self.eat("?");
            if !self.eat(":") {
                return None;
            }
            let t = self.ty()?;
            ps.push(if optional { Ty::Js } else { t });
            self.eat(",");
        }
        Some(ps)
    }

    // hibana の型の書き方にする。オブジェクト型は名前の無い構造体を作る
    fn hib(&mut self, t: &Ty) -> String {
        match t {
            Ty::Int => "int".into(),
            Ty::Str => "str".into(),
            Ty::Bool => "bool".into(),
            Ty::Unit => "unit".into(),
            Ty::Js => "js".into(),
            Ty::Vec(e) => format!("Vec<{}>", self.hib(e)),
            Ty::Fn(ps, r) => {
                let ps: Vec<String> = ps.iter().map(|p| self.hib(p)).collect();
                match &**r {
                    Ty::Promise(r) => format!("(({}) -> {} ! Async)", ps.join(", "), self.hib(r)),
                    r => format!("({}) -> {}", ps.join(", "), self.hib(r)),
                }
            }
            Ty::Promise(_) => "js".into(),
            Ty::Obj(fs) => {
                self.anon += 1;
                let name = format!("Object{}", self.anon);
                self.struct_decl(&name, fs);
                name
            }
            Ty::Named(n) if self.names.contains(n) => n.clone(),
            Ty::Named(_) => "js".into(),
        }
    }

    fn struct_decl(&mut self, name: &str, fs: &[(String, Ty)]) {
        let fs: Vec<String> = fs
            .iter()
            .map(|(n, t)| format!("{n}: {}", self.hib(t)))
            .collect();
        self.structs
            .push(format!("struct {name} {{ {} }}\n", fs.join(", ")));
    }

    fn function(&mut self) {
        let name = self.next();
        if self.peek() == "<" || !self.eat("(") {
            // ジェネリクスは読めない
            self.skip_decl();
            return;
        }
        let ps = self.params();
        let ret = if self.eat(":") {
            self.ty()
        } else {
            Some(Ty::Js)
        };
        // Promise<T> を返す関数は、T を返す Async の効果の関数にする
        let (ret, is_async) = match ret {
            Some(Ty::Promise(t)) => (Some(*t), true),
            r => (r, false),
        };
        if self.peek() == "{" {
            self.skip_group();
        }
        self.eat(";");
        let (Some(ps), Some(ret)) = (ps, ret) else {
            return;
        };
        let ps: Vec<String> = ps
            .iter()
            .enumerate()
            .map(|(i, p)| format!("a{i}: {}", self.hib(p)))
            .collect();
        let ret = self.hib(&ret);
        let eff = if is_async { " ! Async" } else { "" };
        self.fns
            .push(format!("    fn {name}({}) -> {ret}{eff}\n", ps.join(", ")));
    }
}

// TypeScript の型宣言 src を読み、モジュール path（use に書いた経路）を使う hibana の本文を作る
pub fn module_text(src: &str, path: &str) -> R<String> {
    let t = lex(src);
    // 先に、このファイルで宣言した型の名前を集める（宣言より前で使ってもよいように）
    let names: Vec<String> = t
        .windows(3)
        .filter(|w| w[0] == "export" || w[0] == "declare")
        .filter(|w| matches!(w[1].as_str(), "interface" | "type"))
        .map(|w| w[2].clone())
        .collect();
    let mut p = P {
        t,
        i: 0,
        fns: Vec::new(),
        structs: Vec::new(),
        anon: 0,
        names,
    };
    while p.i < p.t.len() {
        if !p.eat("export") {
            if matches!(p.peek(), "{" | "(" | "[") {
                p.skip_group();
            } else {
                p.i += 1;
            }
            continue;
        }
        p.eat("declare");
        match p.next().as_str() {
            "function" => p.function(),
            "async" if p.eat("function") => p.function(),
            "interface" => {
                let name = p.next();
                while !matches!(p.peek(), "{" | "") {
                    p.i += 1;
                }
                match p.primary() {
                    Some(Ty::Obj(fs)) => p.struct_decl(&name, &fs),
                    _ => p.skip_decl(),
                }
            }
            "type" => {
                let name = p.next();
                if !p.eat("=") {
                    p.skip_decl();
                    continue;
                }
                match p.ty() {
                    Some(Ty::Obj(fs)) => p.struct_decl(&name, &fs),
                    Some(t) => {
                        let t = p.hib(&t);
                        p.structs.push(format!("type {name} = {t}\n"));
                    }
                    None => {}
                }
                p.skip_decl();
            }
            _ => p.skip_decl(),
        }
    }
    if p.fns.is_empty() && p.structs.is_empty() {
        return Err(format!("{path} の型宣言から、export した関数や型を読めなかった").into());
    }
    let mut out = p.structs.concat();
    out += &format!("extern {path:?} {{\n{}}}\n", p.fns.concat());
    Ok(out)
}
