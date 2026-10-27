// Language Server（hibana lsp）。標準入出力で JSON-RPC を読み書きする
// 対応: エラーの表示・整形・型の表示（hover）・定義へのジャンプ
// ponytail: 変更のたびに文書全体を検査し直す。大きくなれば差分の検査や検査の間引きを入れる
use crate::lexer::{Tok, lex};
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

// 文書を検査し、エラーと式ごとの型を返す
fn analyze(uri: &str, src: &str) -> (Option<crate::error::Error>, Vec<(usize, usize, String)>) {
    if let Some(dir) = uri_path(uri).parent() {
        crate::parser::set_base_dir(dir);
    }
    crate::typeck::TYPES.with(|t| *t.borrow_mut() = Some(Vec::new()));
    let r = std::panic::catch_unwind(|| -> crate::error::R<()> {
        let prog = crate::parser::parse(&crate::parser::with_uses(src)?)?;
        crate::typeck::check(&prog)?;
        Ok(())
    });
    let types = crate::typeck::TYPES.with(|t| t.borrow_mut().take()).unwrap_or_default();
    let err = match r {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(_) => Some("型検査器が異常終了した（hibana の誤り）".into()),
    };
    (err, types)
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
            Tok::Punct("fn" | "type" | "effect" | "macro" | "syntax") => true,
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
            if matches!(w[0].tok, Tok::Punct("fn" | "type" | "effect" | "macro" | "syntax")) {
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
    let mut types: HashMap<String, Vec<(usize, usize, String)>> = HashMap::new();
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
                let (err, ts) = analyze(&uri, &text);
                diagnostics(&uri, &text, err);
                types.insert(uri.clone(), ts);
                docs.insert(uri, text);
                continue;
            }
            "textDocument/didClose" => {
                docs.remove(&uri);
                types.remove(&uri);
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
                match best {
                    Some((a, b, t)) => json!({
                        "contents": { "kind": "markdown", "value": format!("```hibana\n{t}\n```") },
                        "range": { "start": position(src, *a), "end": position(src, *b) },
                    }),
                    None => Value::Null,
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
        let (err, ts) = analyze("file:///tmp/a.hb", src);
        assert!(err.is_none());
        let call = src.find("f(1)").unwrap();
        assert!(ts.iter().any(|(a, b, t)| *a == call && *b == call + 4 && t == "int"));
    }
}
