mod c;
mod error;
mod eval;
mod js;
mod lexer;
mod meta;
mod parser;
mod resolve;
mod smt;
mod ts;
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
    let info = typeck::check(&prog)?;
    Ok(js::emit(&prog, &info))
}

// C に変換する
fn to_c(src: &str) -> error::R<String> {
    let prog = parser::parse(&parser::with_uses(src)?)?;
    typeck::check(&prog)?;
    if let Some(f) = prog.fns.iter().find(|f| f.js.is_some()) {
        return Err(format!(
            "extern の関数 {} は JS ターゲット（hibana js）でだけ使える",
            f.name
        )
        .into());
    }
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
    // use に書いたファイルは、このファイルのディレクトリから探す
    parser::set_base_dir(
        std::path::Path::new(&path)
            .parent()
            .unwrap_or(std::path::Path::new(".")),
    );
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
            .filter(|p| p.extension().is_some_and(|x| x == "hb"))
            .collect();
        names.sort();
        for p in names {
            let src = std::fs::read_to_string(&p).unwrap();
            // extern を使う例はインタプリタでは動かないので、js_extern で確かめる
            if src.contains(".ts\"") || src.contains(".js\"") || src.contains("\nextern") {
                continue;
            }
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
            .filter(|p| p.extension().is_some_and(|x| x == "hb"))
            .collect();
        names.sort();
        for p in names {
            let src = std::fs::read_to_string(&p).unwrap();
            // extern を使う例はインタプリタでは動かないので、js_extern で確かめる
            if src.contains(".ts\"") || src.contains(".js\"") || src.contains("\nextern") {
                continue;
            }
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

    // use で JavaScript と TypeScript のモジュールを使う例を、Node.js で実行する
    #[test]
    fn js_extern() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        crate::parser::set_base_dir(&dir);
        let src = std::fs::read_to_string(dir.join("js_interop.hb")).unwrap();
        // 出力した JS からの相対経路（./js/util.ts）で読み込めるよう、examples の中に書き出す
        let js = dir.join(format!("js_interop_{}.mjs", std::process::id()));
        std::fs::write(&js, super::to_js(&src).unwrap()).unwrap();
        let r = std::process::Command::new("node")
            .arg(&js)
            .output()
            .unwrap();
        std::fs::remove_file(&js).unwrap();
        assert_eq!(
            String::from_utf8(r.stdout).unwrap(),
            "50\nHIBANA!\n18\n3\n3\nHello, world\n6\n20\n[3, 5]\n"
        );
        let e = out(&src).unwrap_err();
        assert!(e.contains("JS ターゲット"), "{e}");
        let e = super::to_c(&src).unwrap_err();
        assert!(e.contains("JS ターゲット"), "{e}");
        // 型宣言が無いモジュールは型が分からないので使えない
        let e = out("use \"./nope.js\"\nfn main() {}").unwrap_err();
        assert!(e.contains("型宣言 ./nope.d.ts が無い"), "{e}");
        let e = out("use \"node:path\"\nfn main() {}").unwrap_err();
        assert!(e.contains("@types/node"), "{e}");
        // npm のパッケージは node_modules の package.json の types から型宣言を読む
        let tmp = std::env::temp_dir().join(format!("hibana-npm-{}", std::process::id()));
        let pkg = tmp.join("node_modules/fake-pkg");
        std::fs::create_dir_all(pkg.join("dist")).unwrap();
        std::fs::write(pkg.join("package.json"), r#"{ "name": "fake-pkg", "type": "module", "main": "dist/index.js", "types": "dist/index.d.ts" }"#).unwrap();
        std::fs::write(
            pkg.join("dist/index.js"),
            "export function twice(n) { return n * 2; }\n",
        )
        .unwrap();
        std::fs::write(
            pkg.join("dist/index.d.ts"),
            "export declare function twice(n: number): number;\n",
        )
        .unwrap();
        crate::parser::set_base_dir(&tmp);
        let js = super::to_js("use \"fake-pkg\"\nfn main() { print(fake_pkg.twice(21)) }").unwrap();
        std::fs::write(tmp.join("main.mjs"), js).unwrap();
        let r = std::process::Command::new("node")
            .arg(tmp.join("main.mjs"))
            .output()
            .unwrap();
        std::fs::remove_dir_all(&tmp).unwrap();
        assert_eq!(
            String::from_utf8(r.stdout).unwrap(),
            "42\n",
            "{}",
            String::from_utf8_lossy(&r.stderr)
        );
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
            "<div class=\"card\">Hello Alice!<Icon name=\"star\"></Icon></div>\n200 <h1>Welcome</h1>\n200 user 42\n404 Not Found\nweb.Response { status: 200, content_type: \"text/plain\", body: \"stub(ACCOUNT): post 7 by bob\" }\n"
        );
        // 閉じタグが合わなければマクロがコンパイルエラーにする
        let e = out(&src.replace("</Icon>", "</Star>")).unwrap_err();
        assert!(e.contains("閉じタグ Star が Icon と合わない"), "{e}");
        // ルートの本体からは、マクロが導入した parts は見えない（衛生性）
        let e = out(&src.replace("web.ok(\"user \" + id)", "web.ok(\"user \" + parts[0])"))
            .unwrap_err();
        assert!(e.contains("未定義の変数 parts"), "{e}");
        let e = out("use nothing\nfn main() {}").unwrap_err();
        assert!(e.contains("ライブラリ nothing が無い"), "{e}");
        // 名前空間: use web は修飾して使い、利用者の同じ名前とは衝突しない
        let ok = "fn ok(n: int) -> int { n + 1 }\nfn main() { print(ok(1))\n print(web.ok(\"a\").body) }";
        assert_eq!(out(&format!("use web\n{ok}")).unwrap(), "2\na\n");
        let e = out("use web\nfn main() { print(ok(\"a\").body) }").unwrap_err();
        assert!(e.contains("未定義の関数 ok"), "{e}");
        // use web.* はすべて、use web.{..} は挙げた名前だけ修飾なしで使える
        let all = "fn main() { let r = Request { method: \"GET\", path: \"/\" }\n print(ok(r.path).status) }";
        assert_eq!(out(&format!("use web.*\n{all}")).unwrap(), "200\n");
        assert_eq!(
            out(&format!("use web.{{ok, Request}}\n{all}")).unwrap(),
            "200\n"
        );
        let e = out("use web.{ok}\nfn main() { print(html(\"a\").body) }").unwrap_err();
        assert!(e.contains("未定義の関数 html"), "{e}");
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

    // 効果の型検査。3つの実行方式で同じ結果になることは examples/effects.hb で確かめる
    #[test]
    fn effects() {
        let src = include_str!("../examples/effects.hb");
        assert_eq!(
            out(src).unwrap(),
            "each a\neach b\n[log] find 1\nuser1\n[log] find 0\nnot found: 0\n[log] find 2\nuser2\n3\n"
        );
        let p = "effect Log { fn log(msg: str) }\neffect Ask { fn ask() -> int }\neffect Fail { fn fail(m: str) -> never }\n";
        for (src, msg) in [
            (
                "fn main() { log(\"x\") }",
                "! Log を書くか、handle で処理する",
            ),
            (
                "fn f() ! Log { log(\"x\") }\nfn main() { f() }",
                "! Log を書くか",
            ),
            ("fn main() ! Log { }", "Async だけ"),
            (
                "fn main() { handle log(\"a\") { log(m) => { resume()\nprint(m) } } }",
                "resume は腕の最後に置く",
            ),
            (
                "fn main() { let x = handle ask() + 1 { ask() => resume(\"s\") } }",
                "int が必要だが str",
            ),
            (
                "fn main() { handle fail(\"a\") { fail(m) => resume() } }",
                "never の操作は resume できない",
            ),
            (
                "fn main() { let x = handle 1 { ask() => resume(2) } }",
                "効果 Ask を起こしていない",
            ),
            (
                "fn f() ! Log { let g = fn() { log(\"a\") } }\nfn main() {}",
                "! Log を書くか",
            ),
            (
                "fn f() ! Log { let g = log }\nfn main() {}",
                "値として使えない",
            ),
            (
                "fn twice(f: (int) -> int, x: int) -> int { f(f(x)) }\nfn n(x: int) -> int ! Log { x }\nfn main() { handle twice(n, 1) { log(m) => resume() } }",
                "(int) -> int が必要だが (int) -> int ! Log",
            ),
            ("fn g() -> never { g() }\nfn main() {}", "効果の操作だけ"),
            (
                "fn f() -> int { handle { return 1 } { ask() => resume(2) } }\nfn main() {}",
                "return を使えない",
            ),
        ] {
            let e = out(&format!("{p}{src}")).unwrap_err();
            assert!(e.contains(msg), "{src} => {e}");
        }
        // 腕の中の操作は外側の handle が処理し、resume しない腕は handle の値になる
        let src = format!(
            "{p}fn main() {{\n let x = handle {{ handle {{ fail(\"in\")\n 1 }} {{ fail(m) => {{ log(m)\n 2 }} }} }} {{ log(m) => {{ print(\"log \" + m)\n resume() }} }}\n print(x) }}"
        );
        assert_eq!(out(&src).unwrap(), "log in\n2\n");
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
