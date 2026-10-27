// フォーマッタ。トークンの並びと改行は変えず、字下げ・行内の空白・空行・コメントの位置だけを整える
// ponytail: 行の長さで改行の位置は変えない。変えるなら構文木から出力し直す
use crate::error::R;
use crate::lexer::{Tok, Token, lex};

const OPEN: &[&str] = &["(", "[", "{"];
const CLOSE: &[&str] = &[")", "]", "}"];
// 前後に空白を置く演算子
// ponytail: + - * / % は元の空白のまま（syntax の Route* などの後置と区別できない）
const BIN: &[&str] = &["=", "==", "!=", "<=", ">=", "&&", "||", "=>", "->"];
// 新しい宣言や文を始めるキーワード（続きの行にしない）
const STARTS: &[&str] = &[
    "fn", "let", "type", "syntax", "macro", "effect", "use", "module", "extern", "partial", "return",
    "trait", "impl", "struct", "enum",
];

fn punct(t: &Token) -> &str {
    match &t.tok {
        Tok::Punct(p) => p,
        _ => "",
    }
}

// 値の終わりになるトークン
fn ends_value(t: &Token) -> bool {
    matches!(
        t.tok,
        Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::Char(_) | Tok::Ident(_)
    ) || matches!(punct(t), ")" | "]" | "}" | "true" | "false")
}

// i 番目から始まる JSX の要素（<tag ... </tag>）の最後のトークンの番号
// JSX の本文は空白も文字列になるので、要素はソースのまま写す
fn jsx_end(toks: &[Token], i: usize) -> Option<usize> {
    let starts = |k: usize| {
        punct(&toks[k]) == "<"
            && matches!(toks.get(k + 1).map(|t| &t.tok), Some(Tok::Ident(_)))
            && !toks[k + 1].gap_before
    };
    if !starts(i) || (i > 0 && (ends_value(&toks[i - 1]) || punct(&toks[i - 1]) == ">")) {
        return None;
    }
    let mut depth = 0;
    let mut k = i;
    while k < toks.len() {
        if starts(k) {
            depth += 1;
        } else if punct(&toks[k]) == "</" {
            depth -= 1;
            if depth == 0 {
                return (k + 2 < toks.len()).then_some(k + 2);
            }
        }
        k += 1;
    }
    None
}

// トークンの間のコメント (前の改行の数, 綴り) と、最後のコメントの後の改行の数
fn comments(gap: &str) -> (Vec<(usize, &str)>, usize) {
    let mut out = Vec::new();
    let mut nl = 0;
    let mut i = 0;
    while i < gap.len() {
        let rest = &gap[i..];
        if rest.starts_with("//") {
            let n = rest.find('\n').unwrap_or(rest.len());
            out.push((nl, rest[..n].trim_end()));
            nl = 0;
            i += n;
        } else if rest.starts_with("/*") {
            let n = rest.find("*/").map_or(rest.len(), |k| k + 2);
            out.push((nl, &rest[..n]));
            nl = 0;
            i += n;
        } else {
            let c = rest.chars().next().unwrap();
            if c == '\n' {
                nl += 1;
            }
            i += c.len_utf8();
        }
    }
    (out, nl)
}

pub fn format(src: &str) -> R<String> {
    let toks = lex(src)?;
    let mut out = String::new();
    // 開いた括弧ごとの (中身の字下げの深さ, $ の直後の括弧か)。$( と ${ の中は元の空白のまま
    let mut stack: Vec<(usize, bool)> = vec![(0, false)];
    // 続きの行（演算子で終わった行の次や | . で始まる行）を始めた括弧の深さ
    let mut cont: Option<usize> = None;
    // syntax の宣言の中か。宣言の続きの行は、演算子で終わっていなくても続きの行にする
    let mut in_syntax = false;
    let mut line_indent = 0;
    // 今の行の初めに開いていた括弧の数。それより多く開いていれば、この行で開いた括弧の中
    let mut line_base = 1;
    let mut prev: Option<usize> = None;
    let mut prev_end = 0;
    // 直前のトークンが前後に空白を置く演算子か、$( を閉じた ) か
    let mut prev_bin = false;
    let mut prev_rep = false;
    let mut i = 0;
    let newline = |out: &mut String, blank: bool, indent: usize| {
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
        if blank {
            out.push('\n');
        }
        out.push_str(&"    ".repeat(indent));
    };
    while i < toks.len() {
        let t = &toks[i];
        let gap = &src[prev_end..t.span.start.offset];
        let (cs, nl) = comments(gap);
        let after_open = prev.is_some_and(|p| OPEN.contains(&punct(&toks[p])));
        let is_close = CLOSE.contains(&punct(t));
        let mut fresh_line = prev.is_none();
        for (k, (cnl, c)) in cs.iter().enumerate() {
            if *cnl == 0 && k == 0 && prev.is_some() {
                out.push(' ');
            } else if !out.is_empty() {
                let top = stack.last().unwrap().0;
                newline(&mut out, *cnl >= 2 && !after_open, top);
                line_indent = top;
                line_base = stack.len();
            }
            out.push_str(c);
            fresh_line = false;
        }
        if prev.is_none() && punct(t) == "syntax" {
            in_syntax = true;
        }
        if t.tok == Tok::Eof {
            break;
        }
        if nl > 0 || (!cs.is_empty() && cs.last().unwrap().1.starts_with("//")) {
            // 行頭に並んだ閉じ括弧のうち、一番外側の括弧を開いた行の深さにする
            let closers = toks[i..]
                .iter()
                .enumerate()
                .take_while(|(k, x)| CLOSE.contains(&punct(x)) && (*k == 0 || !x.gap_before))
                .count();
            if is_close {
                stack.pop();
            }
            let mut top = stack[stack.len() - closers.max(1)].0;
            if closers == 0 {
                top = stack.last().unwrap().0;
            }
            let blank = nl >= 2 && !is_close && (!after_open || !cs.is_empty());
            let starts = prev.is_some_and(|p| BIN.contains(&punct(&toks[p])))
                || matches!(punct(t), "|" | ".");
            if blank || STARTS.contains(&punct(t)) {
                in_syntax = punct(t) == "syntax";
            }
            let keeps = in_syntax
                && cont == Some(stack.len())
                && closers == 0
                && !STARTS.contains(&punct(t));
            cont = (starts || keeps).then_some(stack.len());
            if cont.is_some() {
                top += 1;
            }
            newline(&mut out, blank, top);
            line_indent = top;
            line_base = stack.len();
        } else if !fresh_line {
            let p = &toks[prev.unwrap()];
            let (pp, c) = (punct(p), punct(t));
            let had = !gap.is_empty();
            let bin = BIN.contains(&c);
            let splice = stack.last().unwrap().1;
            let space = if prev_rep
                || (splice && (matches!(pp, "(" | "{") || matches!(c, ")" | "}")))
            {
                had
            } else if matches!(c, "," | ")" | "]" | ";" | ":") || matches!(pp, "(" | "[") {
                false
            } else if pp == "}" && c == "else" {
                true
            } else if pp == "," {
                !matches!(c, "*" | "+" | "?")
            } else if c == "{" {
                pp != "$"
            } else if pp == "{" {
                c != "}"
            } else if c == "}" || bin || prev_bin {
                true
            } else {
                had
            };
            if is_close {
                stack.pop();
                line_base = line_base.min(stack.len());
            }
            if space {
                out.push(' ');
            }
        }
        // JSX の要素はソースのまま
        if let Some(end) = jsx_end(&toks, i) {
            out.push_str(&src[t.span.start.offset..toks[end].span.end.offset]);
            prev = Some(end);
            prev_end = toks[end].span.end.offset;
            prev_bin = false;
            prev_rep = false;
            i = end + 1;
            continue;
        }
        out.push_str(&src[t.span.start.offset..t.span.end.offset]);
        let c = punct(t);
        prev_rep = false;
        if OPEN.contains(&c) {
            let level = if stack.len() > line_base {
                stack.last().unwrap().0
            } else {
                line_indent + 1
            };
            let splice = c != "[" && prev.is_some_and(|p| punct(&toks[p]) == "$");
            stack.push((level, splice));
        } else if is_close {
            prev_rep = c == ")" && rep_close(&toks, i);
        }
        prev_bin = BIN.contains(&c);
        prev = Some(i);
        prev_end = t.span.end.offset;
        i += 1;
    }
    let mut out: String = out.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
    out = out.trim().to_string();
    out.push('\n');
    // 整形でトークンや改行の位置が変わっていないか確かめる（変わると文の区切りが変わる）
    let after = lex(&out)?;
    let key = |ts: &[Token]| -> Vec<(Tok, bool)> {
        ts.iter().map(|t| (t.tok.clone(), t.nl_before)).collect()
    };
    if key(&toks) != key(&after) {
        return Err("整形でトークンの並びが変わった（フォーマッタの誤り）".into());
    }
    Ok(out)
}

// i 番目の ) が $( を閉じるか
fn rep_close(toks: &[Token], i: usize) -> bool {
    let mut depth = 0;
    for k in (0..i).rev() {
        match punct(&toks[k]) {
            ")" | "]" | "}" => depth += 1,
            "(" | "[" | "{" if depth > 0 => depth -= 1,
            "(" => return k > 0 && punct(&toks[k - 1]) == "$",
            "[" | "{" => return false,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn layout() {
        let src = "fn add(a:int,b :int)->int{   a+b }  // 足す\n\n\n\nfn main(){\nlet x=add( 1,2 )\n      if x==3 {print( \"ok\" )}else{\n  print(\"ng\")\n}\n    /* c */\n  let ok = x == 3 ||\n  x == 4\n  print(<p>a   b {x}</p>)\n  print(web.json(f([\n    1\n    ])))\n\n}\n";
        let want = "fn add(a:int, b:int) -> int { a+b } // 足す\n\nfn main() {\n    let x = add(1, 2)\n    if x == 3 { print(\"ok\") } else {\n        print(\"ng\")\n    }\n    /* c */\n    let ok = x == 3 ||\n        x == 4\n    print(<p>a   b {x}</p>)\n    print(web.json(f([\n        1\n    ])))\n}\n";
        assert_eq!(format(src).unwrap(), want);
    }

    // 例とライブラリは整形済みで、もう一度整形しても変わらない
    #[test]
    fn examples_are_formatted() {
        for dir in ["examples", "lib"] {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.extension().is_some_and(|x| x == "hb") {
                    let src = std::fs::read_to_string(&p).unwrap();
                    assert_eq!(format(&src).unwrap(), src, "{}", p.display());
                }
            }
        }
    }
}
