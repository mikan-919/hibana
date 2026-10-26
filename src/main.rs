mod c;
mod error;
mod eval;
mod js;
mod lexer;
mod meta;
mod parser;
mod smt;
mod typeck;

use std::io::Write;

fn run(src: &str, out: &mut dyn Write) -> error::R<eval::Value> {
    let prog = parser::parse(&parser::with_uses(src)?)?;
    typeck::check(&prog)?;
    Ok(eval::run(&prog, out)?)
}

// JavaScript に変換する
fn to_js(src: &str) -> error::R<String> {
    let prog = parser::parse(&parser::with_uses(src)?)?;
    typeck::check(&prog)?;
    Ok(js::emit(&prog))
}

// C に変換する
fn to_c(src: &str) -> error::R<String> {
    let prog = parser::parse(&parser::with_uses(src)?)?;
    typeck::check(&prog)?;
    Ok(c::emit(&prog))
}

// C に変換し、cc でネイティブの実行ファイル out を作る
fn build(src: &str, out: &std::path::Path) -> error::R<()> {
    let code = to_c(src)?;
    let c_path = out.with_extension("c");
    std::fs::write(&c_path, code).map_err(|e| e.to_string())?;
    let r = std::process::Command::new(std::env::var("CC").unwrap_or("cc".into()))
        .args(["-O2", "-w", "-o"])
        .arg(out)
        .arg(&c_path)
        .status()
        .map_err(|e| format!("cc を起動できない: {e}"))?;
    if !r.success() {
        return Err(format!("cc が失敗した（{}）", c_path.display()).into());
    }
    Ok(())
}

// ソースの該当箇所に下線を引いて表示するためのエラー
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{msg}")]
struct Report {
    msg: String,
    #[source_code]
    src: miette::NamedSource<String>,
    #[label("ここ")]
    span: Option<miette::SourceSpan>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, path) = match args.as_slice() {
        [p] => ("run", p.clone()),
        [cmd, p] if cmd == "js" || cmd == "c" || cmd == "build" => (cmd.as_str(), p.clone()),
        _ => {
            eprintln!(
                "使い方: hibana <ファイル>（実行） | hibana js <ファイル>（JavaScript を出力） | hibana c <ファイル>（C を出力） | hibana build <ファイル>（ネイティブの実行ファイルを作る）"
            );
            std::process::exit(2);
        }
    };
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(2);
    });
    let r = match cmd {
        "js" => to_js(&src).map(|s| print!("{s}")),
        "c" => to_c(&src).map(|s| print!("{s}")),
        "build" => build(&src, &std::path::Path::new(&path).with_extension("")),
        _ => run(&src, &mut std::io::stdout()).map(|_| ()),
    };
    if let Err(e) = r {
        let report = Report {
            msg: e.msg,
            // 読み込んだライブラリの中のエラーも指せるよう、ライブラリを足したソースを渡す
            src: miette::NamedSource::new(&path, parser::with_uses(&src).unwrap_or(src.clone())),
            span: e.span.map(Into::into),
        };
        eprintln!("{:?}", miette::Report::new(report));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    fn out(src: &str) -> crate::error::R<String> {
        let mut buf = Vec::new();
        super::run(src, &mut buf)?;
        Ok(String::from_utf8(buf).unwrap())
    }

    // すべての例について、JavaScript に変換して Node.js で実行した結果がインタプリタと同じか
    #[test]
    fn js_backend_matches_interpreter() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let tmp = std::env::temp_dir().join(format!("hibana-js-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        names.sort();
        for p in names {
            let src = std::fs::read_to_string(&p).unwrap();
            let want = out(&src).unwrap();
            let js = tmp.join(p.file_stem().unwrap()).with_extension("js");
            std::fs::write(&js, super::to_js(&src).unwrap()).unwrap();
            let r = std::process::Command::new("node")
                .arg(&js)
                .output()
                .expect("node が要る（nix develop の中で実行する）");
            assert_eq!(
                String::from_utf8(r.stdout).unwrap(),
                want,
                "{}",
                p.display()
            );
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    // すべての例について、C に変換してネイティブにビルドした実行結果がインタプリタと同じか
    #[test]
    fn native_backend_matches_interpreter() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let tmp = std::env::temp_dir().join(format!("hibana-c-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        names.sort();
        for p in names {
            let src = std::fs::read_to_string(&p).unwrap();
            let want = out(&src).unwrap();
            let exe = tmp.join(p.file_stem().unwrap());
            super::build(&src, &exe).unwrap();
            let r = std::process::Command::new(&exe).output().unwrap();
            assert_eq!(
                String::from_utf8(r.stdout).unwrap(),
                want,
                "{}",
                p.display()
            );
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn macros_example() {
        let src = include_str!("../examples/macros.hb");
        assert_eq!(out(src).unwrap(), "1\n81\n11\n-3\na\n10\nb\n20\n");
    }

    #[test]
    fn web_macros() {
        let src = include_str!("../examples/web.hb");
        assert_eq!(
            out(src).unwrap(),
            "<div class=\"card\">Hello Alice!<Icon name=\"star\"></Icon></div>\n200 <h1>Welcome</h1>\n200 user 42\n404 Not Found\nResponse { status: 200, content_type: \"text/plain\", body: \"stub(ACCOUNT): post 7 by bob\" }\n"
        );
        // 閉じタグが合わなければマクロがコンパイルエラーにする
        let e = out(&src.replace("</Icon>", "</Star>")).unwrap_err();
        assert!(e.contains("閉じタグ Star が Icon と合わない"), "{e}");
        // ルートの本体からは、マクロが導入した parts は見えない（衛生性）
        let e = out(&src.replace("ok(\"user \" + id)", "ok(\"user \" + parts[0])")).unwrap_err();
        assert!(e.contains("未定義の変数 parts"), "{e}");
        let e = out("use nothing\nfn main() {}").unwrap_err();
        assert!(e.contains("ライブラリ nothing が無い"), "{e}");
    }

    #[test]
    fn hygiene() {
        // マクロの中の tmp は呼び出し側の tmp と別の変数になる
        let src = r#"
macro swap ("swap" a:Ident b:Ident) -> Stmt {
    quote {
        let tmp = $a
        $a = $b
        $b = tmp
    }
}
fn main() {
    let mut tmp = 1
    let mut y = 2
    swap tmp y
    print(tmp)
    print(y)
}
"#;
        assert_eq!(out(src).unwrap(), "2\n1\n");
    }

    #[test]
    fn example_runs() {
        let src = include_str!("../examples/fib.hb");
        assert_eq!(
            out(src).unwrap(),
            "0\n1\n1\n2\n3\n5\n8\n13\n21\n34\n55\ntrue\n"
        );
    }

    #[test]
    fn errors() {
        for (src, msg) in [
            ("fn main() { let x = 1; x = 2; }", "mut でない"),
            ("fn main() { let x: int = true; }", "int が必要"),
            ("fn f() -> int { true } fn main() {}", "戻り値"),
            ("fn main() { if 1 { } }", "if の条件"),
            ("fn main() { print(1 / 0); }", "0 で除算"),
            ("fn main() { y; }", "未定義の変数"),
            ("fn main() { 1 + ; }", "式が必要"),
            ("fn main() { let x = 1 print(x) }", "文の区切り"),
            ("fn main() { let x = quote { 1 } }", "マクロ本体の中でだけ"),
        ] {
            let e = out(src).unwrap_err();
            assert!(e.contains(msg), "{src} => {e}");
        }
    }
}
