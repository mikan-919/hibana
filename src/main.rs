mod c;
mod error;
mod eval;
mod fmt;
mod js;
mod lexer;
mod lsp;
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
    eval::run(&prog, out)
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
    // JavaScript のモジュールの関数は使えない（大域の関数 extern { } は、呼ぶと実行時エラー）
    if let Some(f) = prog
        .fns
        .iter()
        .find(|f| f.js.as_ref().is_some_and(|m| !m.is_empty()))
    {
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
    // 下線に付ける説明。メッセージの一番内側の部分（"関数 f: 引数 x: int が必要だが str" の "int が必要だが str"）
    #[label("{label}")]
    span: Option<miette::SourceSpan>,
    label: String,
    #[help]
    help: Option<String>,
    #[label("ここで起こしている")]
    origin: Option<miette::SourceSpan>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["lsp"] {
        lsp::serve();
        return;
    }
    let (cmd, path) = match args.as_slice() {
        [p] => ("run", p.clone()),
        [cmd, p] if cmd == "js" || cmd == "c" || cmd == "build" || cmd == "fmt" => (cmd.as_str(), p.clone()),
        _ => {
            eprintln!(
                "使い方: hibana <ファイル>（実行） | hibana js <ファイル>（JavaScript を出力） | hibana c <ファイル>（C を出力） | hibana build <ファイル>（ネイティブの実行ファイルを作る） | hibana fmt <ファイル>（整形して書き戻す） | hibana lsp（Language Server を標準入出力で動かす）"
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
        "fmt" => fmt::format(&src).map(|s| {
            if s != src {
                std::fs::write(&path, s).unwrap_or_else(|e| {
                    eprintln!("{path}: {e}");
                    std::process::exit(2);
                });
            }
        }),
        "build" => build(&src, &std::path::Path::new(&path).with_extension("")),
        _ => run(&src, &mut std::io::stdout()).map(|_| ()),
    };
    if let Err(e) = r {
        let label = e.msg.rsplit(": ").next().unwrap_or("ここ").to_string();
        // 文字列が必要な所の数値・真偽値には to_str を提案する
        let to_str = ["str が必要だが int", "str が必要だが bool"]
            .iter()
            .any(|s| e.msg.contains(s))
            .then(|| "to_str(値) で文字列にする".to_string());
        let report = Report {
            help: e.help.clone().or(to_str),
            origin: e.origin.map(Into::into),
            label,
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
            // extern を使う例はインタプリタでは動かないので、js_extern で確かめる。サーバーは web_server で確かめる
            if src.contains(".ts\"")
                || src.contains(".js\"")
                || src.contains("\nextern")
                || src.contains("web.serve(")
            {
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
            // extern を使う例はインタプリタでは動かないので、js_extern で確かめる。サーバーは web_server で確かめる
            if src.contains(".ts\"")
                || src.contains(".js\"")
                || src.contains("\nextern")
                || src.contains("web.serve(")
            {
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
            "50\nHIBANA!\n18\n3\n3\nHello, world\n6\n20\n[3, 5]\n[\"a2a\", \"b4b\"]\n[20, 40]\n"
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

    // examples/server.hb を Node.js の HTTP サーバーと Cloudflare Workers の fetch として動かす
    #[test]
    fn web_server() {
        use std::io::{BufRead, Read};
        let src = include_str!("../examples/server.hb");
        let tmp = std::env::temp_dir().join(format!("hibana-server-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        // port 0 で空いている番号を使い、表示された URL に要求を送る
        let js = tmp.join("server.mjs");
        std::fs::write(
            &js,
            super::to_js(&src.replace("web.serve(8080, app)", "web.serve(0, app)")).unwrap(),
        )
        .unwrap();
        let mut child = std::process::Command::new("node")
            .arg(&js)
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
        assert_eq!(lines.next().unwrap().unwrap(), "found alice");
        let url = lines.next().unwrap().unwrap();
        let addr = url.trim_start_matches("http://");
        let send = |req: String| {
            let mut s = std::net::TcpStream::connect(addr).unwrap();
            std::io::Write::write_all(&mut s, req.as_bytes()).unwrap();
            let mut r = String::new();
            s.read_to_string(&mut r).unwrap();
            r
        };
        // 確かめる前にサーバーを止める（失敗しても node が残らないように）
        let get = |path: &str| {
            send(format!(
                "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
            ))
        };
        let rs = [
            get("/users/bob"),
            get("/users/zed"),
            get("/"),
            send("POST /users HTTP/1.1\r\nHost: x\r\nContent-Length: 16\r\nConnection: close\r\n\r\n{\"name\":\"carol\"}".into()),
            send("POST /users HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into()),
            get("/users"),
            send("POST /echo?from=me HTTP/1.1\r\nHost: x\r\nX-Agent: t\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi".into()),
        ];
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(
            rs[0].starts_with("HTTP/1.1 200") && rs[0].contains("found bob"),
            "{}",
            rs[0]
        );
        assert!(rs[1].starts_with("HTTP/1.1 404"), "{}", rs[1]);
        assert!(rs[2].contains("content-type: text/html"), "{}", rs[2]);
        assert!(rs[3].starts_with("HTTP/1.1 201"), "{}", rs[3]);
        assert!(rs[4].starts_with("HTTP/1.1 400"), "{}", rs[4]);
        assert!(
            rs[5].contains(r#"{"users":["alice","bob","carol"]}"#),
            "{}",
            rs[5]
        );
        assert!(
            rs[6].contains(r#"{"from":"me","agent":"t","body":"hi"}"#),
            "{}",
            rs[6]
        );
        // Workers では export default { fetch } を呼ぶ
        let js = tmp.join("worker.mjs");
        std::fs::write(
            &js,
            super::to_js(&src.replace("web.serve(8080, app)", "web.worker(app)")).unwrap(),
        )
        .unwrap();
        let r = std::process::Command::new("node")
            .args(["--input-type=module", "-e", "const m = await import(process.argv[1]); const r = await m.default.fetch(new Request('https://example.com/users/alice')); console.log(r.status, await r.text());"])
            .arg(&js)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(r.stdout).unwrap(),
            "found alice\n200 found alice\n"
        );
        std::fs::remove_dir_all(&tmp).unwrap();
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
        let all = "fn main() { let r = request(\"GET\", \"/\")\n print(ok(r.path).status) }";
        assert_eq!(out(&format!("use web.*\n{all}")).unwrap(), "200\n");
        assert_eq!(
            out(&format!("use web.{{ok, request}}\n{all}")).unwrap(),
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
            "[\"u7\", \"not found: 0\"]\neach a\neach b\n[log] find 1\nuser1\n[log] find 0\nnot found: 0\n[log] find 2\nuser2\n3\n"
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
            // 効果の多相: E は渡した関数の効果になり、呼ぶ側で処理する必要がある
            (
                "fn ap<E: Effect>(f: () -> unit ! E) ! E { f() }\nfn n() ! Log { log(\"a\") }\nfn main() { ap(n) }",
                "! Log を書くか",
            ),
            (
                "fn ap<E: Effect>(f: () -> unit ! E) ! E { log(\"a\") }\nfn main() {}",
                "! Log を書くか",
            ),
            (
                "fn ap<E: Effect>(f: (int) -> unit ! E) ! E { f(1) }\nfn main() { ap(fn(x) { log(\"a\") }) }",
                "! Log を書くか",
            ),
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

    // 型エラーの提案と、効果を起こした操作の位置
    #[test]
    fn error_hints() {
        let e = out("fn main() {\n let count = 1\n print(cuont)\n}").unwrap_err();
        assert_eq!(e.help.as_deref(), Some("count のことか"));
        let e = out("fn main() { prnt(1) }").unwrap_err();
        assert_eq!(e.help.as_deref(), Some("print のことか"));
        let src = "effect Log {\n fn log(m: str)\n}\nfn n() ! Log { log(\"a\") }\nfn main() { n() }";
        let e = out(src).unwrap_err();
        assert_eq!(e.help.as_deref(), Some("関数 n の型が ! Log を持つ"));
        let (at, n) = e.origin.unwrap();
        assert_eq!(&src[at..at + n], "log(\"a\")");
    }

    // 実行時エラーの位置。インタプリタはエラーに位置を付け、JS と C はメッセージに行と列を足す
    #[test]
    fn runtime_error_positions() {
        let src = "fn half(x: int, y: int) -> int {\n    x / y\n}\nfn main() {\n    print(half(1, 0))\n}\n";
        let e = out(src).unwrap_err();
        assert_eq!(e.span, Some((37, 5)), "{e:?}");
        let tmp = std::env::temp_dir().join(format!("hibana-pos-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let js = tmp.join("d.mjs");
        std::fs::write(&js, super::to_js(src).unwrap()).unwrap();
        let r = std::process::Command::new("node")
            .arg(&js)
            .output()
            .unwrap();
        let want = "実行時エラー: 0 で除算した（2 行 5 列）\n";
        assert_eq!(String::from_utf8(r.stderr).unwrap(), want);
        let exe = tmp.join("d");
        super::build(src, &exe).unwrap();
        let r = std::process::Command::new(&exe).output().unwrap();
        assert_eq!(String::from_utf8(r.stderr).unwrap(), want);
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    // よくある条件（routes の添字、fib の停止性など）は Z3 なしで証明できる
    #[test]
    fn builtin_prover() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        crate::parser::set_base_dir(&dir);
        for p in std::fs::read_dir(&dir).unwrap() {
            let p = p.unwrap().path();
            let name = p.file_name().unwrap().to_str().unwrap().to_string();
            // smt.hb は Z3 の例、struct.hb は掛け算の条件を Z3 で示す
            if !name.ends_with(".hb") || name == "smt.hb" || name == "struct.hb" {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            crate::smt::without_z3(|| super::to_js(&src)).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
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
