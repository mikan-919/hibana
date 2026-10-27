// Language Server（hibana lsp）。標準入出力で JSON-RPC を読み書きする
// 対応: エラーの表示・整形・hover（宣言の形・コメント・式の型）・定義へのジャンプ・補完・色付け（semantic tokens）
// ponytail: 変更のたびに文書全体を検査し直す。大きくなれば差分の検査や検査の間引きを入れる
use crate::index::{Index, Kind};
use crate::lexer::{Tok, Token, lex};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, Write};

fn read_msg(r: &mut impl BufRead) -> Option<Value> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            len = v.trim().parse().ok();
        }
    }
    let mut buf = vec![0; len?];
    r.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn send(v: Value) {
    let s = v.to_string();
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{s}", s.len());
    let _ = out.flush();
}

// file:// の URI からパス
fn uri_path(uri: &str) -> std::path::PathBuf {
    let s = uri.strip_prefix("file://").unwrap_or(uri);
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Ok(n) = u8::from_str_radix(s.get(i + 1..i + 3).unwrap_or(""), 16)
        {
            out.push(n);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned().into()
}

// バイト位置から LSP の位置（行, UTF-16 の列）
fn position(src: &str, off: usize) -> Value {
    let off = off.min(src.len());
    let before = &src[..off];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |k| k + 1);
    json!({ "line": line, "character": before[start..].encode_utf16().count() })
}

fn offset(src: &str, pos: &Value) -> usize {
    let (line, ch) = (
        pos["line"].as_u64().unwrap_or(0) as usize,
        pos["character"].as_u64().unwrap_or(0) as usize,
    );
    let mut off = 0;
    for (k, l) in src.split_inclusive('\n').enumerate() {
        if k == line {
            let mut u = 0;
            for (b, c) in l.char_indices() {
                if u >= ch {
                    return off + b;
                }
                u += c.len_utf16();
            }
            return off + l.trim_end_matches('\n').len();
        }
        off += l.len();
    }
    src.len()
}

type Types = Vec<(usize, usize, String)>;

// 文書を検査し、エラーと式ごとの型と、名前の表（構文解析できた時だけ）を返す
fn analyze(uri: &str, src: &str) -> (Option<crate::error::Error>, Types, Option<Index>) {
    if let Some(dir) = uri_path(uri).parent() {
        crate::parser::set_base_dir(dir);
    }
    crate::typeck::TYPES.with(|t| *t.borrow_mut() = Some(Vec::new()));
    let mut index = None;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> crate::error::R<()> {
        let combined = crate::parser::with_uses(src)?;
        let prog = crate::parser::parse(&combined)?;
        index = Some(crate::index::build(src, &combined, &prog));
        crate::typeck::check(&prog)?;
        Ok(())
    }));
    let types = crate::typeck::TYPES.with(|t| t.borrow_mut().take()).unwrap_or_default();
    let err = match r {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(_) => Some("型検査器が異常終了した（hibana の誤り）".into()),
    };
    (err, types, index)
}

// 位置にある名前。モジュールで修飾されていれば web.json の形にする。(名前, 先頭, 終わり)
fn name_at(toks: &[Token], ix: &Index, off: usize) -> Option<(String, usize, usize)> {
    let k = toks.iter().position(|t| {
        matches!(t.tok, Tok::Ident(_)) && t.span.start.offset <= off && off <= t.span.end.offset
    })?;
    let Tok::Ident(n) = &toks[k].tok else { unreachable!() };
    let sp = (toks[k].span.start.offset, toks[k].span.end.offset);
    if k >= 2
        && toks[k - 1].tok == Tok::Punct(".")
        && let Tok::Ident(m) = &toks[k - 2].tok
        && ix.modules.contains(m)
    {
        return Some((format!("{m}.{n}"), sp.0, sp.1));
    }
    Some((n.clone(), sp.0, sp.1))
}

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "else", "while", "for", "in", "return", "match", "type", "effect",
    "handle", "resume", "use", "macro", "syntax", "quote", "partial", "extern", "trait", "impl", "struct",
    "enum", "true",
    "false",
];

// 補完の候補
fn completion(src: &str, off: usize, ix: &Index, types: &Types) -> Value {
    let b = src.as_bytes();
    let mut p = off.min(src.len());
    while p > 0 && (b[p - 1].is_ascii_alphanumeric() || b[p - 1] == b'_') {
        p -= 1;
    }
    let item = |label: &str, kind: u32, detail: &str, doc: &str| {
        json!({ "label": label, "kind": kind, "detail": detail, "documentation": doc })
    };
    let kind_no = |k: Kind| match k {
        Kind::Function | Kind::Macro => 3,
        Kind::Type => 22,
        Kind::Ctor => 20,
        Kind::Effect => 8,
    };
    let mut out = Vec::new();
    if p > 0 && b[p - 1] == b'.' {
        let dot = p - 1;
        let mut q = dot;
        while q > 0 && (b[q - 1].is_ascii_alphanumeric() || b[q - 1] == b'_') {
            q -= 1;
        }
        let recv = &src[q..dot];
        if ix.modules.contains(recv) {
            // モジュールの関数・型
            for s in &ix.syms {
                if let Some(rest) = s.name.strip_prefix(&format!("{recv}.")) {
                    out.push(item(rest, kind_no(s.kind), &s.detail, &s.doc));
                }
            }
        } else if let Some(t) = types
            .iter()
            .filter(|(_, e, _)| *e == dot)
            .max_by_key(|(a, e, _)| e - a)
            .map(|x| x.2.clone())
            .or_else(|| var_type(src, recv, q, types))
        {
            // 構造体のフィールド。型の名前から、型引数と修飾を除いて引く
            let t = t.as_str();
            let base = t.split('<').next().unwrap_or(t);
            let base = base.rsplit('.').next().unwrap_or(base);
            for f in ix.fields.get(base).into_iter().flatten() {
                out.push(item(f, 5, t, ""));
            }
        }
        return json!(out);
    }
    for k in KEYWORDS {
        out.push(item(k, 14, "", ""));
    }
    for m in &ix.modules {
        out.push(item(m, 9, "モジュール", ""));
    }
    for s in ix.syms.iter().filter(|s| !s.name.contains('.')) {
        out.push(item(&s.name, kind_no(s.kind), &s.detail, &s.doc));
    }
    // ponytail: off より前の let・引数・for の名前を、スコープを見ずに足す
    if let Ok(toks) = lex(&src[..p]) {
        let mut seen = std::collections::HashSet::new();
        for (k, w) in toks.windows(2).enumerate() {
            let local = matches!(w[0].tok, Tok::Punct("let" | "mut" | "for"))
                || (matches!(w[0].tok, Tok::Punct("(" | ","))
                    && toks.get(k + 2).is_some_and(|t| t.tok == Tok::Punct(":")));
            if let (true, Tok::Ident(n)) = (local, &w[1].tok)
                && seen.insert(n.clone())
            {
                out.push(item(n, 6, "", ""));
            }
        }
    }
    json!(out)
}

// before より前の宣言から、変数 name の型。書いた型（x: T）か、let の右辺の式の型
fn var_type(src: &str, name: &str, before: usize, types: &Types) -> Option<String> {
    let toks = lex(&src[..before]).ok()?;
    let k = (0..toks.len().saturating_sub(1)).rev().find(|&k| {
        toks[k + 1].tok == Tok::Ident(name.to_string())
            && matches!(toks[k].tok, Tok::Punct("let" | "mut" | "(" | ","))
    })?;
    let after = toks.get(k + 2)?;
    if after.tok == Tok::Punct(":") {
        // 型の終わりは、括弧の外の = , ) か改行
        let start = toks.get(k + 3)?.span.start.offset;
        let mut depth = 0;
        let mut end = start;
        for t in &toks[k + 3..] {
            match t.tok {
                Tok::Punct("<" | "(" | "[") => depth += 1,
                Tok::Punct(">" | ")" | "]") if depth > 0 => depth -= 1,
                Tok::Punct("=" | "," | ")") if depth == 0 => break,
                _ if t.nl_before && t.span.start.offset > start => break,
                _ => {}
            }
            end = t.span.end.offset;
        }
        return Some(src[start..end].to_string());
    }
    let init = toks.get(k + 3).filter(|_| after.tok == Tok::Punct("="))?.span.start.offset;
    types
        .iter()
        .filter(|(a, _, _)| *a == init)
        .max_by_key(|(a, e, _)| e - a)
        .map(|x| x.2.clone())
}

// semantic tokens の種類。並びの番号で送る
const TOKEN_TYPES: &[&str] = &["namespace", "type", "enumMember", "function", "macro", "interface", "property"];

// 名前のトークンを、名前の表から種類分けして LSP の semantic tokens の並び（相対位置）にする
fn semantic_tokens(src: &str, ix: &Index) -> Value {
    let Ok(toks) = lex(src) else {
        return json!({ "data": [] });
    };
    let mut data = Vec::new();
    let (mut last_line, mut last_col) = (0u32, 0u32);
    for (k, t) in toks.iter().enumerate() {
        let Tok::Ident(n) = &t.tok else { continue };
        let prev = |d: usize| k.checked_sub(d).map(|i| &toks[i].tok);
        let after_dot = prev(1) == Some(&Tok::Punct(".")) && !t.gap_before;
        let ty = if toks.get(k + 1).is_some_and(|x| x.tok == Tok::Punct(".")) && ix.modules.contains(n) {
            Some(0)
        } else {
            let name = match prev(2) {
                Some(Tok::Ident(m)) if after_dot && ix.modules.contains(m) => format!("{m}.{n}"),
                _ if after_dot => {
                    // モジュールでない値のフィールド
                    String::new()
                }
                _ => n.clone(),
            };
            if name.is_empty() {
                Some(6)
            } else {
                ix.get(&name).map(|s| match s.kind {
                    Kind::Type => 1,
                    Kind::Ctor => 2,
                    Kind::Function => 3,
                    Kind::Macro => 4,
                    Kind::Effect => 5,
                })
            }
        };
        let Some(ty) = ty else { continue };
        let decl = matches!(prev(1), Some(Tok::Punct("fn" | "type" | "struct" | "enum" | "effect" | "macro" | "trait")));
        let pos = position(src, t.span.start.offset);
        let (line, col) = (pos["line"].as_u64().unwrap() as u32, pos["character"].as_u64().unwrap() as u32);
        let dcol = if line == last_line { col - last_col } else { col };
        data.extend([line - last_line, dcol, n.len() as u32, ty, decl as u32]);
        (last_line, last_col) = (line, col);
    }
    json!({ "data": data })
}

fn diagnostics(uri: &str, src: &str, err: Option<crate::error::Error>) {
    let mut ds = Vec::new();
    if let Some(e) = err {
        // ライブラリの中のエラーは、ファイルの先頭に出す
        let (a, n) = e.span.filter(|(a, _)| *a < src.len()).unwrap_or((0, 0));
        let mut msg = e.msg.clone();
        if let Some(h) = &e.help {
            msg += &format!("\nhelp: {h}");
        }
        let mut d = json!({
            "range": { "start": position(src, a), "end": position(src, a + n) },
            "severity": 1,
            "source": "hibana",
            "message": msg,
        });
        if let Some((oa, on)) = e.origin.filter(|(a, _)| *a < src.len()) {
            d["relatedInformation"] = json!([{
                "location": { "uri": uri, "range": { "start": position(src, oa), "end": position(src, oa + on) } },
                "message": "ここで起こしている",
            }]);
        }
        ds.push(d);
    }
    send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": ds },
    }));
}

// off にある名前の、ファイルの中の宣言の位置（fn・type・effect・macro・syntax・let・引数）
// ponytail: 名前だけで探す（スコープは見ない）。let と引数は off より前の最後のもの。ライブラリの中の宣言には飛べない
fn definition(src: &str, off: usize) -> Option<(usize, usize)> {
    let toks = lex(src).ok()?;
    // カーソルが名前の直後にあっても、その名前を指す
    let name = toks.iter().find_map(|t| match &t.tok {
        Tok::Ident(n) if t.span.start.offset <= off && off <= t.span.end.offset => Some(n),
        _ => None,
    })?;
    let mut found = None;
    for (k, w) in toks.windows(2).enumerate() {
        let decl = match &w[0].tok {
            Tok::Punct("fn" | "type" | "struct" | "enum" | "effect" | "macro" | "syntax") => true,
            Tok::Punct("let" | "mut" | "for") => w[1].span.start.offset <= off,
            // 引数（( か , の後の 名前 :）
            Tok::Punct("(" | ",") => {
                w[1].span.start.offset <= off
                    && toks.get(k + 2).is_some_and(|t| t.tok == Tok::Punct(":"))
            }
            _ => false,
        };
        if decl && w[1].tok == Tok::Ident(name.clone()) {
            let sp = (w[1].span.start.offset, w[1].span.end.offset);
            if matches!(w[0].tok, Tok::Punct("fn" | "type" | "struct" | "enum" | "effect" | "macro" | "syntax")) {
                return Some(sp);
            }
            found = Some(sp);
        }
    }
    found
}

pub fn serve() {
    let mut input = std::io::stdin().lock();
    let mut docs: HashMap<String, String> = HashMap::new();
    let mut types: HashMap<String, Types> = HashMap::new();
    // 最後に構文解析できた時の名前の表（書きかけで構文エラーの間も補完に使う）
    let mut indexes: HashMap<String, Index> = HashMap::new();
    while let Some(msg) = read_msg(&mut input) {
        let method = msg["method"].as_str().unwrap_or("");
        let id = msg.get("id").cloned();
        let p = &msg["params"];
        let uri = p["textDocument"]["uri"].as_str().unwrap_or("").to_string();
        let result = match method {
            "initialize" => json!({
                "capabilities": {
                    "textDocumentSync": 1,
                    "hoverProvider": true,
                    "definitionProvider": true,
                    "documentFormattingProvider": true,
                    "completionProvider": { "triggerCharacters": ["."] },
                    "semanticTokensProvider": {
                        "legend": { "tokenTypes": TOKEN_TYPES, "tokenModifiers": ["declaration"] },
                        "full": true,
                    },
                },
                "serverInfo": { "name": "hibana" },
            }),
            "shutdown" => Value::Null,
            "exit" => return,
            "textDocument/didOpen" | "textDocument/didChange" => {
                let text = if method == "textDocument/didOpen" {
                    p["textDocument"]["text"].as_str()
                } else {
                    p["contentChanges"][0]["text"].as_str()
                };
                let text = text.unwrap_or("").to_string();
                let (err, ts, ix) = analyze(&uri, &text);
                diagnostics(&uri, &text, err);
                // 構文解析できなかった時は、前の型と名前の表を使い続ける（書きかけの x. の補完のため）
                if let Some(ix) = ix {
                    indexes.insert(uri.clone(), ix);
                    types.insert(uri.clone(), ts);
                }
                docs.insert(uri, text);
                continue;
            }
            "textDocument/didClose" => {
                docs.remove(&uri);
                types.remove(&uri);
                indexes.remove(&uri);
                send(json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": { "uri": uri, "diagnostics": [] },
                }));
                continue;
            }
            "textDocument/hover" => {
                let src = docs.get(&uri).map_or("", |s| s.as_str());
                let off = offset(src, &p["position"]);
                // 位置を含む一番短い式の型
                let best = types.get(&uri).and_then(|ts| {
                    ts.iter()
                        .filter(|(a, b, _)| *a <= off && off < *b && *b <= src.len())
                        .min_by_key(|(a, b, _)| b - a)
                });
                let ix = indexes.get(&uri).cloned().unwrap_or_default();
                let toks = lex(src).unwrap_or_default();
                // 宣言された名前なら宣言の形とコメント、そうでなければ式の型
                let sym = name_at(&toks, &ix, off).and_then(|(n, a, b)| Some((ix.get(&n)?.clone(), a, b)));
                let fence = "```";
                match (sym, best) {
                    (Some((s, a, b)), _) => {
                        let mut v = format!("{fence}hibana\n{}\n{fence}", s.detail);
                        if !s.doc.is_empty() {
                            v += &format!("\n\n{}", s.doc);
                        }
                        json!({
                            "contents": { "kind": "markdown", "value": v },
                            "range": { "start": position(src, a), "end": position(src, b) },
                        })
                    }
                    (None, Some((a, b, t))) => json!({
                        "contents": { "kind": "markdown", "value": format!("{fence}hibana\n{t}\n{fence}") },
                        "range": { "start": position(src, *a), "end": position(src, *b) },
                    }),
                    _ => Value::Null,
                }
            }
            "textDocument/definition" => {
                let src = docs.get(&uri).map_or("", |s| s.as_str());
                match definition(src, offset(src, &p["position"])) {
                    Some((a, b)) => json!({
                        "uri": uri,
                        "range": { "start": position(src, a), "end": position(src, b) },
                    }),
                    None => Value::Null,
                }
            }
            "textDocument/completion" => {
                let src = docs.get(&uri).map_or("", |s| s.as_str());
                let ix = indexes.get(&uri).cloned().unwrap_or_default();
                let ts = types.get(&uri).cloned().unwrap_or_default();
                completion(src, offset(src, &p["position"]), &ix, &ts)
            }
            "textDocument/semanticTokens/full" => {
                let src = docs.get(&uri).map_or("", |s| s.as_str());
                semantic_tokens(src, &indexes.get(&uri).cloned().unwrap_or_default())
            }
            "textDocument/formatting" => {
                let src = docs.get(&uri).map_or("", |s| s.as_str());
                match crate::fmt::format(src) {
                    Ok(out) if out != src => json!([{
                        "range": { "start": position(src, 0), "end": position(src, src.len()) },
                        "newText": out,
                    }]),
                    _ => json!([]),
                }
            }
            _ => {
                // 知らない要求にはエラーを返す。通知は無視する
                if let Some(id) = id {
                    send(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("未対応: {method}") },
                    }));
                }
                continue;
            }
        };
        if let Some(id) = id {
            send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_and_definition() {
        let src = "fn f(x: int) -> int { x }\nfn main() {\n let y = f(1)\n print(\"あ\" + to_str(y))\n}\n";
        let use_at = src.rfind("y)").unwrap();
        assert_eq!(position(src, use_at), json!({ "line": 3, "character": 20 }));
        assert_eq!(offset(src, &json!({ "line": 3, "character": 20 })), use_at);
        let (a, _) = definition(src, use_at).unwrap();
        assert_eq!(a, src.find("y =").unwrap());
        let (a, _) = definition(src, src.find("f(1)").unwrap()).unwrap();
        assert_eq!(a, 3);
        let (a, _) = definition(src, src.find("x }").unwrap()).unwrap();
        assert_eq!(a, 5);
        let (err, ts, _) = analyze("file:///tmp/a.hb", src);
        assert!(err.is_none());
        let call = src.find("f(1)").unwrap();
        assert!(ts.iter().any(|(a, b, t)| *a == call && *b == call + 4 && t == "int"));
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    #[test]
    fn completes_modules_fields_and_names() {
        let src = "use web\nstruct P { x: int, y: int }\nfn main() {\n let p = P { x: 1, y: 2 }\n print(p.x)\n}\n";
        let (err, ts, ix) = analyze("file:///tmp/a.hb", src);
        assert!(err.is_none());
        let ix = ix.unwrap();
        let labels = |v: Value| -> Vec<String> {
            v.as_array().unwrap().iter().map(|i| i["label"].as_str().unwrap().to_string()).collect()
        };
        // 書きかけの文書でも、前に解析した型と名前の表を使う
        let typing = src.replace("print(p.x)", "print(p.");
        let at = typing.find("p.").unwrap() + 2;
        assert_eq!(labels(completion(&typing, at, &ix, &ts)), ["x", "y"]);
        let typing = src.replace("print(p.x)", "web.");
        let at = typing.find("web.").unwrap() + 4;
        assert!(labels(completion(&typing, at, &ix, &ts)).contains(&"json".to_string()));
        let all = labels(completion(src, src.find("print").unwrap(), &ix, &ts));
        assert!(all.contains(&"p".to_string()) && all.contains(&"web".to_string()));
        // P は型の宣言（種類 1、宣言の印 1）
        let data = semantic_tokens(src, &ix)["data"].clone();
        assert_eq!(data.as_array().unwrap()[..5], [json!(1), json!(7), json!(1), json!(1), json!(1)]);
    }
}
