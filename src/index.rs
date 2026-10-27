// LSP の補完・色付け・hover に使う名前の表。構文解析の結果と、ライブラリを足したソースから作る
use crate::lexer::{Tok, Token, lex};
use crate::parser::Program;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Function,
    Type,
    Ctor,
    Effect,
    Macro,
}

#[derive(Clone, Debug)]
pub struct Sym {
    // 利用者のファイルで書く名前（修飾が要るなら web.json）
    pub name: String,
    pub kind: Kind,
    // 宣言の形（fn json(body: str) -> Response など）
    pub detail: String,
    // 宣言の直前の // コメント
    pub doc: String,
}

#[derive(Clone, Debug, Default)]
pub struct Index {
    pub syms: Vec<Sym>,
    // 修飾して使うモジュールの名前（use web の web）
    pub modules: HashSet<String>,
    // 型の名前（修飾なし）から、構造体のフィールドの名前
    pub fields: HashMap<String, Vec<String>>,
}

impl Index {
    pub fn get(&self, name: &str) -> Option<&Sym> {
        self.syms.iter().find(|s| s.name == name)
    }
}

// 宣言のキーワードから始まるトークンの並びを、宣言の形の文字列にする（本体の { か行末まで）
fn signature(src: &str, toks: &[Token], i: usize) -> String {
    let start = toks[i].span.start.offset;
    let mut depth = 0i32;
    let mut end = toks[i].span.end.offset;
    for (k, t) in toks.iter().enumerate().skip(i) {
        if k > i && t.nl_before && depth == 0 {
            break;
        }
        match &t.tok {
            Tok::Punct("(" | "[") => depth += 1,
            Tok::Punct(")" | "]") => depth -= 1,
            Tok::Punct("{") if depth == 0 && k > i + 1 => break,
            Tok::Eof => break,
            _ => {}
        }
        end = t.span.end.offset;
    }
    src[start..end].to_string()
}

// offset の行の直前に続く // コメントの行
fn doc_above(src: &str, offset: usize) -> String {
    let line_start = src[..offset].rfind('\n').map_or(0, |k| k + 1);
    let mut lines: Vec<&str> = src[..line_start]
        .lines()
        .rev()
        .map_while(|l| l.trim().strip_prefix("//"))
        .map(str::trim)
        // 区切りの行（// ---- JSON ----）は説明でない
        .filter(|c| !c.starts_with("----"))
        .collect();
    lines.reverse();
    lines.join("\n")
}

// 利用者のファイルの use から、修飾なしで使えるモジュール（use std.*）と、修飾して使うモジュールを求める
fn uses(src: &str) -> (HashSet<String>, HashSet<String>) {
    let (mut glob, mut qualified) = (HashSet::new(), HashSet::new());
    let Ok(toks) = lex(src) else {
        return (glob, qualified);
    };
    for (k, w) in toks.windows(2).enumerate() {
        if w[0].tok != Tok::Punct("use") {
            continue;
        }
        let m = match &w[1].tok {
            Tok::Ident(n) => n.clone(),
            Tok::Str(s) => crate::parser::module_name(s),
            _ => continue,
        };
        let star = toks.get(k + 2).is_some_and(|t| t.tok == Tok::Punct("."))
            && toks.get(k + 3).is_some_and(|t| t.tok == Tok::Punct("*"));
        if star {
            glob.insert(m);
        } else {
            qualified.insert(m);
        }
    }
    (glob, qualified)
}

pub fn build(src: &str, combined: &str, prog: &Program) -> Index {
    let (glob, qualified) = uses(src);
    // 利用者のファイルで書く名前
    let written = |q: &str| -> String {
        match q.split_once('.') {
            Some((m, n)) if glob.contains(m) => n.to_string(),
            _ => q.to_string(),
        }
    };
    // 宣言の形とコメントを、(修飾した名前) ごとにソースから集める
    let mut decls: HashMap<String, (String, String, &'static str)> = HashMap::new();
    if let Ok(toks) = lex(combined) {
        let mut module = String::new();
        for i in 0..toks.len().saturating_sub(1) {
            let kw = match &toks[i].tok {
                Tok::Punct(p @ ("fn" | "type" | "struct" | "enum" | "effect" | "macro" | "syntax" | "trait")) => *p,
                Tok::Punct("module") => {
                    if let Tok::Ident(m) = &toks[i + 1].tok {
                        module = m.clone();
                    }
                    continue;
                }
                _ => continue,
            };
            let Tok::Ident(n) = &toks[i + 1].tok else {
                continue;
            };
            // partial fn は partial から
            let at = if i > 0 && toks[i - 1].tok == Tok::Punct("partial") { i - 1 } else { i };
            let q = crate::parser::qual(&module, n);
            decls.entry(q).or_insert((
                signature(combined, &toks, at),
                doc_above(combined, toks[at].span.start.offset),
                kw,
            ));
        }
    }
    let mut syms = Vec::new();
    let effects: HashSet<&str> = prog.fns.iter().filter_map(|f| f.op.as_deref()).collect();
    let info = |q: &str| decls.get(q).cloned().unwrap_or_default();
    // impl から作った関数（Show#impl0 など）は利用者が呼ばない
    for f in prog.fns.iter().filter(|f| !f.name.contains('#')) {
        let (detail, doc, _) = info(&f.name);
        syms.push(Sym { name: written(&f.name), kind: Kind::Function, detail, doc });
    }
    let mut fields = HashMap::new();
    for d in &prog.datas {
        let (detail, doc, _) = info(&d.name);
        let kind = if effects.contains(d.name.as_str()) { Kind::Effect } else { Kind::Type };
        syms.push(Sym { name: written(&d.name), kind, detail: detail.clone(), doc: doc.clone() });
        let short = d.name.rsplit('.').next().unwrap().to_string();
        if let [c] = d.ctors.as_slice() {
            fields.insert(short.clone(), c.fields.iter().map(|f| f.0.clone()).collect());
        }
        // 構造体の構築子（Name{}）は名前で呼ばない
        for c in d.ctors.iter().filter(|c| !c.name.ends_with("{}")) {
            let fs: Vec<&str> = c.fields.iter().map(|f| f.0.as_str()).collect();
            let shape = if fs.is_empty() { String::new() } else { format!("({})", fs.join(", ")) };
            syms.push(Sym {
                name: written(&c.name),
                kind: Kind::Ctor,
                detail: format!("{}{shape} : {short}", c.name.rsplit('.').next().unwrap()),
                doc: doc.clone(),
            });
        }
    }
    // マクロは展開されて構文木に残らないので、ソースから。マクロの名前はモジュールで修飾しない
    for (q, (detail, doc, kw)) in &decls {
        if *kw == "macro" {
            let name = q.rsplit('.').next().unwrap().to_string();
            syms.push(Sym { name, kind: Kind::Macro, detail: detail.clone(), doc: doc.clone() });
        }
    }
    Index { syms, modules: qualified, fields }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols() {
        let src = "use std.*\nuse web\n// 2 倍にする\nfn double(x: int) -> int { x * 2 }\nstruct P { x: int, y: int }\nfn main() { print(double(1)) }\n";
        let combined = crate::parser::with_uses(src).unwrap();
        let prog = crate::parser::parse(&combined).unwrap();
        let ix = build(src, &combined, &prog);
        let d = ix.get("double").unwrap();
        assert_eq!(d.detail, "fn double(x: int) -> int");
        assert_eq!(d.doc, "2 倍にする");
        // use std.* の関数は修飾なし、use web の関数は修飾付き
        assert_eq!(ix.get("map").unwrap().kind, Kind::Function);
        assert!(ix.get("web.json").is_some());
        assert_eq!(ix.get("Some").unwrap().kind, Kind::Ctor);
        assert_eq!(ix.get("routes").unwrap().kind, Kind::Macro);
        assert_eq!(ix.fields["P"], ["x", "y"]);
        assert!(ix.modules.contains("web"));
        // トレイトのメソッドは関数、impl から作った関数は出さない
        assert_eq!(ix.get("show").unwrap().detail, "fn show(x: A) -> str");
        assert!(ix.syms.iter().all(|s| !s.name.contains('#')));
    }
}
