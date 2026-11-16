use crate::{error, eval, fmt, lexer, lsp, parser, pkg, typeck};
#[cfg(test)]
use crate::{run, to_js, to_web};

// ソースの該当箇所に下線を引いて表示するためのエラー
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{msg}")]
struct Report {
    msg: String,
    #[source_code]
    src: miette::NamedSource<String>,
    // 下線に付ける説明。メッセージの一番内側の部分（"関数 f: 引数 x: int が必要だが String" の "int が必要だが String"）
    #[label("{label}")]
    span: Option<miette::SourceSpan>,
    label: String,
    #[help]
    help: Option<String>,
    #[label("ここで起こしている")]
    origin: Option<miette::SourceSpan>,
}

// 型検査と変換は構文木を再帰でたどるので、深く入れ子になったコードのために大きなスタックで動かす
pub fn main() {
    let t = std::thread::Builder::new().stack_size(STACK).spawn(run_cli).unwrap();
    if t.join().is_err() {
        std::process::exit(101);
    }
}

const STACK: usize = 256 << 20;

fn script_usage() -> ! {
    eprintln!("使い方: hibana <build|dev> [-- 引数...]");
    std::process::exit(2);
}

// 識別子の後は解釈しない。先頭の区切り -- だけを取り除き、残りを実行先へ渡す。
fn run_dlx(args: &[String]) -> Result<i32, String> {
    let usage = "使い方: hibana dlx [--bin コマンド | --rev Gitの版] <npm:名前|cargo:名前|hibana:GitのURL|GitのURL> [--] 引数...";
    let mut bin = None;
    let mut rev = None;
    let mut i = 0;
    while let Some(flag) = args.get(i).filter(|s| s.starts_with('-')) {
        if flag == "--" {
            i += 1;
            break;
        }
        let slot = match flag.as_str() {
            "--bin" => &mut bin,
            "--rev" => &mut rev,
            _ => return Err(format!("不明な dlx オプション: {flag}\n{usage}")),
        };
        if slot.is_some() {
            return Err(format!("{flag} は一度だけ指定できます"));
        }
        let value = args
            .get(i + 1)
            .filter(|s| !s.is_empty() && !s.starts_with('-'))
            .ok_or_else(|| format!("{flag} に値が必要\n{usage}"))?;
        *slot = Some(value.as_str());
        i += 2;
    }
    let spec = args.get(i).filter(|s| !s.is_empty()).ok_or(usage)?;
    let rest = &args[i + 1..];
    let rest = if rest.first().is_some_and(|s| s == "--") {
        &rest[1..]
    } else {
        rest
    };
    if spec.starts_with("npm:") || spec.starts_with("cargo:") {
        if rev.is_some() {
            return Err("--rev は Hibana パッケージ専用です。npm/Cargo は 名前@版 で指定してください".into());
        }
        crate::tools::dlx(spec, bin, rest)
    } else {
        if bin.is_some() {
            return Err("--bin は npm/Cargo パッケージ専用です。Hibana は src/main.hb を実行します".into());
        }
        pkg::dlx(spec, rev, rest)
    }
}

fn update_target<'a>(args: &[&'a str]) -> Result<Option<&'a str>, String> {
    match args {
        [] => Ok(None),
        [name] if !name.is_empty() && !name.starts_with('-') => Ok(Some(*name)),
        _ => Err("使い方: hibana update [依存名|npm:名前|cargo:名前]".into()),
    }
}

fn remove_target<'a>(args: &[&'a str]) -> Result<(bool, &'a str), String> {
    let usage = "使い方: hibana remove [-g|--global] <依存名|npm:名前|cargo:名前>";
    match args {
        [name] if !name.is_empty() && !name.starts_with('-') => Ok((false, *name)),
        [flag, name]
            if (*flag == "-g" || *flag == "--global")
                && !name.is_empty()
                && !name.starts_with('-') =>
        {
            Ok((true, *name))
        }
        [name, flag]
            if (*flag == "-g" || *flag == "--global")
                && !name.is_empty()
                && !name.starts_with('-') =>
        {
            Ok((true, *name))
        }
        _ => Err(usage.into()),
    }
}

fn run_cli() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["lsp"] {
        lsp::serve();
        return;
    }
    // build と dev は、設定したシェルスクリプトを実行する。
    let script = match args.as_slice() {
        [name] if matches!(name.as_str(), "dev" | "build") => Some((name.as_str(), &[][..])),
        [name, sep, rest @ ..] if matches!(name.as_str(), "dev" | "build") && sep == "--" => Some((name.as_str(), rest)),
        [cmd, ..] if matches!(cmd.as_str(), "dev" | "build") => script_usage(),
        _ => None,
    };
    if let Some((name, extra)) = script {
        let code = crate::scripts::run(name, extra).unwrap_or_else(|e| {
            eprintln!("{e}");
            1
        });
        std::process::exit(code);
    }
    // 外部ツールは取得元付き識別子で解決し、通常のコマンド名を実行する。
    if args.first().is_some_and(|s| s == "dlx") {
        let code = run_dlx(&args[1..]).unwrap_or_else(|e| {
            eprintln!("{e}");
            1
        });
        std::process::exit(code);
    }
    if args.first().is_some_and(|s| s == "add") {
        let external = match args.as_slice() {
            [_, flag, spec] if flag == "-g" || flag == "--global" => Some(crate::tools::add(spec, true)),
            [_, spec, flag] if flag == "-g" || flag == "--global" => Some(crate::tools::add(spec, true)),
            [_, spec] if spec.starts_with("npm:") || spec.starts_with("cargo:") => Some(crate::tools::add(spec, false)),
            [_, spec, ..] if spec.starts_with("npm:") || spec.starts_with("cargo:") || spec.starts_with('-') => {
                Some(Err("使い方: hibana add [-g] <npm:名前[@版]|cargo:名前[@版]>".into()))
            }
            _ => None,
        };
        if let Some(result) = external {
            if let Err(e) = result {
                eprintln!("{e}");
                std::process::exit(1);
            }
            return;
        }
    }
    // パッケージのコマンド
    let here = || pkg::root(std::path::Path::new(".")).ok_or(format!("{} が無い（hibana new で作る）", pkg::MANIFEST));
    let r = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["new", name] => Some(pkg::new(name)),
        ["fetch"] => Some(here().and_then(|r| pkg::fetch(&r))),
        ["install"] => Some(crate::tools::install(false)),
        ["install", "--frozen-lockfile"] => Some(crate::tools::install(true)),
        ["install", ..] => Some(Err("使い方: hibana install [--frozen-lockfile]".into())),
        ["update", rest @ ..] => Some(update_target(rest).and_then(|name| match name {
            Some(name) if name.starts_with("npm:") || name.starts_with("cargo:") => {
                crate::tools::update(name)
            }
            name => here().and_then(|r| pkg::update(&r, name)),
        })),
        ["remove", rest @ ..] => Some(remove_target(rest).and_then(|(global, name)| {
            if name.starts_with("npm:") || name.starts_with("cargo:") {
                crate::tools::remove(name, global)
            } else if global {
                Err("-g は npm/Cargo の global shim にだけ指定できます".into())
            } else {
                here().and_then(|r| pkg::remove(&r, name))
            }
        })),
        ["test", files @ ..] => Some(test_cmd(files).and_then(|ok| if ok { Ok(()) } else { Err(String::new()) })),
        ["add", url, tag @ ..] if tag.len() <= 1 => {
            Some(here().and_then(|r| pkg::add(&r, url, tag.first().copied())).map(|n| eprintln!("追加: use {n}")))
        }
        _ => None,
    };
    if let Some(r) = r {
        if let Err(e) = r {
            if !e.is_empty() {
                eprintln!("{e}");
            }
            std::process::exit(1);
        }
        return;
    }
    if let Some(cmd) = args.first().filter(|s| matches!(s.as_str(), "ir" | "wasm" | "glue" | "web" | "js" | "c" | "script" | "run")) {
        eprintln!("不明なサブコマンド: {cmd}");
        std::process::exit(2);
    }
    let (cmd, path) = match args.as_slice() {
        [p] => ("eval", p.clone()),
        [p, sep, _rest @ ..] if sep == "--" => ("eval", p.clone()),
        [cmd, p] if cmd == "fmt" => (cmd.as_str(), p.clone()),
        [p, ..] if p.ends_with(".hb") => ("eval", p.clone()),
        _ => {
            eprintln!(
                "使い方: hibana install [--frozen-lockfile]（依存を取得） | hibana update [依存名|npm:名前|cargo:名前]（依存を更新） | hibana remove [-g] <依存名|npm:名前|cargo:名前>（依存を削除） | hibana <build|dev> [-- 引数...] | hibana <ファイル> [-- 引数...]（実行） | hibana fmt <ファイル>（整形して書き戻す） | hibana lsp（Language Server を標準入出力で動かす） | hibana new <名前> | hibana add <git の URL> [tag] | hibana add [-g] <npm:名前[@版]|cargo:名前[@版]> | hibana dlx [--bin コマンド | --rev Gitの版] <識別子> [--] 引数... | hibana fetch（依存を取ってくる） | hibana test [ファイル]（test を実行する）"
            );
            std::process::exit(2);
        }
    };
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(2);
    });
    // hibana.toml があれば、依存を取ってから（取ってあれば何もしない）コンパイルする
    let dir = std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new("."));
    if let Some(root) = pkg::root(if dir.as_os_str().is_empty() { std::path::Path::new(".") } else { dir }) {
        if let Err(e) = pkg::fetch(&root) {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
    // use に書いたファイルは、このファイルのディレクトリから探す
    parser::set_base_dir(
        std::path::Path::new(&path)
            .parent()
            .unwrap_or(std::path::Path::new(".")),
    );
    let r = match cmd {
        "fmt" => fmt::format(&src).map(|s| {
            if s != src {
                std::fs::write(&path, s).unwrap_or_else(|e| {
                    eprintln!("{path}: {e}");
                    std::process::exit(2);
                });
            }
        }),
        _ => {
            let rest = match args.as_slice() { [_, sep, rest @ ..] if sep == "--" => rest, [_, rest @ ..] => rest, _ => &[] };
            crate::run_with_args(&src, &mut std::io::stdout(), rest).map(|_| ())
        },
    };
    if let Err(e) = r {
        eprintln!("{:?}", report(&path, &src, e));
        std::process::exit(1);
    }
}

// エラーを、ソースの位置に下線を引いて表示する形にする
fn report(path: &str, src: &str, e: error::Error) -> miette::Report {
    {
        let label = e.msg.rsplit(": ").next().unwrap_or("ここ").to_string();
        // 文字列が必要な所の数値・真偽値には to_str を提案する
        let to_str = ["String が必要だが int", "String が必要だが bool"]
            .iter()
            .any(|s| e.msg.contains(s))
            .then(|| "値.to_string() で文字列にする".to_string());
        let report = Report {
            help: e.help.clone().or(to_str),
            origin: e.origin.map(Into::into),
            label,
            msg: e.msg,
            // 読み込んだライブラリの中のエラーも指せるよう、ライブラリを足したソースを渡す
            src: miette::NamedSource::new(path, parser::with_uses(src).unwrap_or(src.to_string())),
            span: e.span.map(Into::into),
        };
        miette::Report::new(report)
    }
}

// hibana test [ファイル...]: test "名前" { } をインタプリタで実行する。
// ファイルを省くと、hibana.toml のあるディレクトリの src と tests の .hb すべて
fn test_cmd(files: &[&str]) -> Result<bool, String> {
    let mut paths: Vec<std::path::PathBuf> = files.iter().map(Into::into).collect();
    if paths.is_empty() {
        let root = pkg::root(std::path::Path::new(".")).ok_or(format!("{} が無い（ファイルを指定する）", pkg::MANIFEST))?;
        pkg::fetch(&root)?;
        for d in ["src", "tests"] {
            for e in std::fs::read_dir(root.join(d)).into_iter().flatten().flatten() {
                if e.path().extension().is_some_and(|x| x == "hb") {
                    paths.push(e.path());
                }
            }
        }
        paths.sort();
    }
    let (mut pass, mut total) = (0, 0);
    for p in &paths {
        let path = p.display().to_string();
        let src = std::fs::read_to_string(p).map_err(|e| format!("{path}: {e}"))?;
        parser::set_base_dir(p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(std::path::Path::new(".")));
        // ponytail: main の無いファイルには空の main を足す（型検査が main を要るため）。main を要らなくするなら型検査に入口を渡す
        let has_main = lexer::lex(&src).is_ok_and(|ts| {
            ts.windows(2).any(|w| w[0].tok == lexer::Tok::Punct("fn") && w[1].tok == lexer::Tok::Ident("main".into()))
        });
        let full = if has_main { src.clone() } else { format!("{src}\nfn main() {{}}\n") };
        let prog = parser::with_uses(&full).and_then(|s| parser::parse(&s)).and_then(|mut prog| {
            typeck::check(&mut prog)?;
            Ok(prog)
        });
        let prog = match prog {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{:?}", report(&path, &full, e));
                total += 1;
                continue;
            }
        };
        for (label, f) in prog.tests.iter().filter(|t| !t.1.contains('.')) {
            total += 1;
            match eval::run_fn(&prog, f, full.len(), &mut std::io::stdout()) {
                Ok(_) => {
                    pass += 1;
                    println!("ok   {label}");
                }
                Err(e) => {
                    println!("失敗 {label}");
                    eprintln!("{:?}", report(&path, &full, e));
                }
            }
        }
    }
    println!("{total} 件中 {pass} 件が成功");
    Ok(pass == total)
}

#[cfg(test)]
mod tests {
    #[test]
    fn update_arguments() {
        assert_eq!(super::update_target(&[]).unwrap(), None);
        assert_eq!(super::update_target(&["foo"]).unwrap(), Some("foo"));
        assert_eq!(
            super::update_target(&["npm:vite"]).unwrap(),
            Some("npm:vite")
        );
        assert_eq!(
            super::update_target(&["cargo:clap"]).unwrap(),
            Some("cargo:clap")
        );
        for args in [
            vec!["foo", "bar"],
            vec!["--frozen-lockfile"],
            vec!["--"],
            vec![""],
        ] {
            assert_eq!(
                super::update_target(&args).unwrap_err(),
                "使い方: hibana update [依存名|npm:名前|cargo:名前]"
            );
        }
    }

    #[test]
    fn remove_arguments() {
        assert_eq!(super::remove_target(&["foo"]).unwrap(), (false, "foo"));
        assert_eq!(super::remove_target(&["npm:vite"]).unwrap(), (false, "npm:vite"));
        assert_eq!(super::remove_target(&["-g", "cargo:clap"]).unwrap(), (true, "cargo:clap"));
        assert_eq!(super::remove_target(&["npm:vite", "--global"]).unwrap(), (true, "npm:vite"));
        for args in [
            vec![],
            vec!["foo", "bar"],
            vec!["--"],
            vec!["-g"],
            vec!["-g", "foo", "extra"],
            vec![""],
        ] {
            assert!(super::remove_target(&args).is_err(), "{args:?}");
        }
    }

    // 階層化した例も、バックエンド比較から漏らさない。
    fn hb_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                files.extend(hb_files(&path));
            } else if kind.is_file() && path.extension().is_some_and(|x| x == "hb") {
                files.push(path);
            }
        }
        files.sort();
        files
    }

    // 3 つの実行方式で確かめるプログラム。テスト用（tests/programs）と、利用者が書く正本（examples）
    fn program_files() -> Vec<std::path::PathBuf> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        ["tests/programs", "examples"].iter().flat_map(|d| hb_files(&root.join(d))).collect()
    }

    // 正本（examples）の .hb の横に同じ名前の .out があれば、インタプリタの出力がその中身と同じか
    #[test]
    fn examples_match_expected_output() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        for p in hb_files(&dir) {
            let Ok(want) = std::fs::read_to_string(p.with_extension("out")) else { continue };
            crate::parser::set_base_dir(p.parent().unwrap());
            let got = out(&std::fs::read_to_string(&p).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert_eq!(got, want, "{}", p.display());
        }
    }

    fn out(src: &str) -> crate::error::R<String> {
        let mut buf = Vec::new();
        super::run(src, &mut buf)?;
        Ok(String::from_utf8(buf).unwrap())
    }

    #[test]
    fn numeric_values() {
        let src = include_str!("../../tests/fixtures/num.hb");
        assert_eq!(out(src).unwrap(), "2.5\n1.5\n1.5\n-0.5\n0.25\n1\n1.5\n2\ntrue\ntrue\ntrue\n[0.5, 2.5]\n-0\n0.00000001\n1000000000000000000000\n");
        for src in [
            "fn main() { let x: int = 0.5 }",
            "fn main() { let x: num[x -> x > 0] = -0.5 }",
            "fn main() { let x: num[x -> x > 9007199254740992] = 9007199254740993 }",
            "struct P { x: num }\nfn f(p: P) -> int[x -> x == 1] { if p.x > 0 && p.x < 1 { 0 } else { 1 } }\nfn main() {}",
            "fn f(x: num) { let mut y: num = x\n if y > 0 { y = -1.0\n let z: num[z -> z > 0] = y } }\nfn main() {}",
            "fn f(x: num) -> num[y -> y >= 1] { if x > 0 { x } else { 1 } }\nfn main() {}",
            "fn f(x: num) -> num[y -> y > x] { x + 1.0 }\nfn main() {}",
        ] { assert!(out(src).is_err(), "{src}"); }
        let math = include_str!("../../tests/fixtures/math.hb");
        assert_eq!(out(math).unwrap(), "3\n1.5\n2\n-3\n5\n");
        for (name, source) in [("num", src), ("math", math)] {
            let path = std::env::temp_dir().join(format!("hibana-{name}-{}.cjs", std::process::id()));
            std::fs::write(&path, super::to_js(source).unwrap()).unwrap();
            let result = std::process::Command::new("node").arg(&path).output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            assert_eq!(String::from_utf8(result.stdout).unwrap(), out(source).unwrap());
            std::fs::remove_file(path).unwrap();
            assert!(crate::to_wasm(source).is_err());
            assert!(crate::to_native(source).is_err());
        }
        for src in ["use std::math -> *\nfn main() { put(sqrt(-1)) }", "use std::math -> *\nfn main() { put(floor(1000000000000000000000.0)) }", "fn main() { put(1.0 / 0.0) }"] {
            assert!(out(src).is_err(), "{src}");
        }
    }

    #[test]
    fn capability_scopes() {
        let src = include_str!("../../tests/programs/types/capabilities.hb");
        assert_eq!(out(src).unwrap(), "created\nplain\n[DEBUG] inner\nrestored\n[OTHER] other\n");
        let missing = src.replace("with Logger(make_logger())", "");
        assert!(out(&missing).unwrap_err().to_string().contains("with Logger が必要"));
        let invalid = src.replace("with Logger(make_logger())", "with Logger(0)");
        assert!(out(&invalid).is_err());
        let escaped = src.replace("fn main() {", "fn main() { emit('outside')\n");
        assert!(out(&escaped).unwrap_err().to_string().contains("with Logger が必要"));
        let multiple = "trait First { fn first(self) -> int }\ntrait Second { fn second(self) -> int }\nimpl First for int { fn first(self) -> int { self } }\nimpl Second for int { fn second(self) -> int { self } }\nfn sum() -> int with First(a), Second(b) { a.first() + b.second() }\nfn main() { with First(2) { with Second(3) { put(sum()) } } }";
        assert_eq!(out(multiple).unwrap(), "5\n");
    }

    #[test]
    fn result_propagation() {
        let src = "use std -> *\nfn get_result(ok: bool) -> Result<int, String> { if ok { Ok(7) } else { Err('bad') } }\nfn twice(ok: bool) -> Result<int, String> { let n = get_result(ok)?\n Ok(n * 2) }\nfn main() { put(twice(true))\n put(twice(false)) }";
        assert_eq!(out(src).unwrap(), "std.Ok(14)\nstd.Err(\"bad\")\n");
        assert!(out("use std -> *\nfn f() -> int { Ok(1)? }\nfn main() {}\n").is_err());
        assert!(out("type Digit = int[x -> 0 <= x && x < 10]\nfn f() -> Result<Digit, String> { Ok(12) }\nfn main() {}\n").is_err());
    }

    // すべての例について、JavaScript に変換して Node.js で実行した結果がインタプリタと同じか
    #[test]
    fn js_backend_matches_interpreter() {
        let tmp = std::env::temp_dir().join(format!("hibana-js-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let names = program_files();
        for p in names {
            // 標準入力を使う正本は tests/first.rs で入力と出力を比較する。
            if p.ends_with("examples/first.hb") { continue; }
            let src = std::fs::read_to_string(&p).unwrap();
            // extern を使う例はインタプリタでは動かないので、js_extern で確かめる
            // main の無いファイル（ライブラリとテスト）は実行しない
            if !src.contains("fn main(")
                || src.contains(".ts'")
                || src.contains(".js'")
                || src.contains("\nextern")
            {
                continue;
            }
            crate::parser::set_base_dir(p.parent().unwrap());
            let want = out(&src).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
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

    // IR を通す 2 つの出力（WASM と Cranelift のネイティブ）の結果が、インタプリタと同じか。
    // IR が未対応のプログラムは飛ばし、対応した数を表示する
    #[test]
    fn ir_backends_match_interpreter() {
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("hibana-ir-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let (mut total, mut native, mut wasm, mut reasons): (usize, Vec<String>, Vec<String>, Vec<String>) = (0, Vec::new(), Vec::new(), Vec::new());
        for p in program_files() {
            let src = std::fs::read_to_string(&p).unwrap();
            // ponytail: first.hb は num を使う。IR に num を追加したら、この拒否の検査を出力比較に含める
            if p.ends_with("examples/first.hb") {
                assert!(crate::to_native(&src).unwrap_err().msg.contains("#num"));
                assert!(crate::to_wasm(&src).unwrap_err().msg.contains("#num"));
                continue;
            }
            if src.contains(".ts'") || src.contains(".js'") || src.contains("\nextern") {
                continue;
            }
            crate::parser::set_base_dir(p.parent().unwrap());
            let Ok(want) = out(&src) else { continue };
            total += 1;
            let name = p.file_stem().unwrap().to_str().unwrap().to_string();
            #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
            match crate::to_native(&src) {
                Ok(bytes) => {
                    let exe = tmp.join(format!("{name}-{total}"));
                    std::fs::write(&exe, bytes).unwrap();
                    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
                    let r = std::process::Command::new(&exe).output().unwrap();
                    assert_eq!(String::from_utf8(r.stdout).unwrap(), want, "ネイティブ: {}", p.display());
                    native.push(name.clone());
                }
                Err(e) => {
                    assert!(e.msg.contains("未対応"), "ネイティブ: {}: {}", p.display(), e.msg);
                    reasons.push(format!("{name}: {}", e.msg));
                }
            }
            match crate::to_wasm(&src) {
                Ok(bytes) => {
                    let path = tmp.join("program.wasm");
                    std::fs::write(&path, bytes).unwrap();
                    let script = "let x; x = new WebAssembly.Instance(new WebAssembly.Module(require('fs').readFileSync(process.argv[1])), {hibana: {write: (fd,p,n) => process.stdout.write(Buffer.from(x.memory.buffer, Number(p), Number(n))), now_ms: () => BigInt(Date.now())}}).exports; x.main();";
                    let got = std::process::Command::new("node").args(["-e", script]).arg(path).output().unwrap();
                    assert!(got.status.success(), "WASM: {}: {}", p.display(), String::from_utf8_lossy(&got.stderr));
                    assert_eq!(String::from_utf8(got.stdout).unwrap(), want, "WASM: {}", p.display());
                    wasm.push(name);
                }
                Err(e) => assert!(e.msg.contains("未対応"), "WASM: {}: {}", p.display(), e.msg),
            }
        }
        std::fs::remove_dir_all(&tmp).unwrap();
        eprintln!("ネイティブ {}/{total}: {native:?}\nWASM {}/{total}: {wasm:?}\n未対応:\n{}", native.len(), wasm.len(), reasons.join("\n"));
        #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
        assert_eq!(native, wasm, "2 つの出力が対応するプログラムは同じ");
        assert!(reasons.is_empty(), "IR が未対応のプログラムがある");
    }

    // 範囲外と 0 による除算は、ネイティブでもメッセージを出して終了コード 1 で終わる
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    #[test]
    fn native_reports_integer_errors() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = std::env::temp_dir().join(format!("hibana-native-err-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let cases = [
            ("fn f(a: int, b: int) -> int { a + b }\nfn main() { put(1)\n put(f(9223372036854775807, 1)) }", "オーバーフロー"),
            ("fn f(a: int, b: int) -> int { a * b }\nfn main() { put(1)\n put(f(-1, -9223372036854775807 - 1)) }", "オーバーフロー"),
            ("fn f(a: int, b: int) -> int { a / b }\nfn main() { put(1)\n put(f(1, 0)) }", "0 で除算した"),
            ("fn f(a: int, b: int) -> int { a % b }\nfn main() { put(1)\n put(f(-9223372036854775807 - 1, -1)) }", "オーバーフロー"),
            ("fn f(a: int, b: int) -> int { a ** b }\nfn main() { put(1)\n put(f(2, -1)) }", "負の指数"),
            ("fn f(a: int, b: int) -> int { a ** b }\nfn main() { put(1)\n put(f(3, 100)) }", "オーバーフロー"),
        ];
        for (i, (src, msg)) in cases.iter().enumerate() {
            let exe = tmp.join(format!("e{i}"));
            std::fs::write(&exe, crate::to_native(src).unwrap()).unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            let r = std::process::Command::new(&exe).output().unwrap();
            assert_eq!(r.status.code(), Some(1), "{src}");
            assert_eq!(String::from_utf8(r.stdout).unwrap(), "1\n", "{src}");
            assert!(String::from_utf8(r.stderr).unwrap().contains(msg), "{src}");
        }
        let src = "fn main() { put(-9223372036854775807 - 1)\n put(0)\n put(-42)\n put(true)\n put('日本語') }";
        let exe = tmp.join("put");
        std::fs::write(&exe, crate::to_native(src).unwrap()).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let r = std::process::Command::new(&exe).output().unwrap();
        assert_eq!(String::from_utf8(r.stdout).unwrap(), out(src).unwrap());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    // use で JavaScript と TypeScript のモジュールを使う例を、Node.js で実行する
    #[test]
    fn js_extern() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs/interop");
        crate::parser::set_base_dir(&dir);
        let src = std::fs::read_to_string(dir.join("js_interop.hb")).unwrap();
        // 出力した JS からの相対経路（./js/util.ts）で読み込めるよう、tests/programs/interop の中に書き出す
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
        // 型宣言が無いモジュールは型が分からないので使えない
        let e = out("use './nope.js'\nfn main() {}").unwrap_err();
        assert!(e.contains("型宣言 ./nope.d.ts が無い"), "{e}");
        let e = out("use 'node:path'\nfn main() {}").unwrap_err();
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
        let js = super::to_js("use 'fake-pkg'\nfn main() { put(fake_pkg.twice(21)) }").unwrap();
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
        let src = include_str!("../../tests/programs/macros/macros.hb");
        assert_eq!(out(src).unwrap(), "1\n81\n11\n-3\na\n10\nb\n20\n");
    }

    #[test]
    fn hygiene() {
        // マクロの中の tmp は呼び出し側の tmp と別の変数になる
        let src = r#"
macro swap ('swap' a:Ident b:Ident) -> Stmt {
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
    put(tmp)
    put(y)
}
"#;
        assert_eq!(out(src).unwrap(), "2\n1\n");
    }

    // 束縛した名前が大域の関数・組み込みの関数・構築子と同じでも、呼び出し側の変数を隠さない（3 つの実行方式は tests/programs の検査で確かめる）
    #[test]
    fn hygiene_bound_names() {
        let src = include_str!("../../tests/programs/macros/hygiene.hb");
        assert_eq!(out(src).unwrap(), "11\n21\n11\n11\n");
    }

    #[test]
    fn example_runs() {
        let src = include_str!("../../tests/programs/basics/fib.hb");
        assert_eq!(
            out(src).unwrap(),
            "0\n1\n1\n2\n3\n5\n8\n13\n21\n34\n55\ntrue\n"
        );
    }

    // 効果の型検査。3つの実行方式で同じ結果になることは tests/programs/effects/effects.hb で確かめる
    #[test]
    fn effects() {
        let src = include_str!("../../tests/programs/effects/effects.hb");
        assert_eq!(
            out(src).unwrap(),
            "[\"u7\", \"not found: 0\"]\neach a\neach b\n[log] find 1\nuser1\n[log] find 0\nnot found: 0\n[log] find 2\nuser2\n3\n"
        );
        let p = "effect Log { fn log(msg: String) }\neffect Ask { fn ask() -> int }\neffect Fail { fn fail(m: String) -> never }\n";
        for (src, msg) in [
            (
                "fn main() { log('x') }",
                "! Log を書くか、handle で処理する",
            ),
            (
                "fn f() ! Log { log('x') }\nfn main() { f() }",
                "! Log を書くか",
            ),
            ("fn main() ! Log { }", "Async だけ"),
            (
                "fn main() { handle log('a') { log(m) -> { resume()\nprint(m) } } }",
                "resume は腕の最後に置く",
            ),
            (
                "fn main() { let x = handle ask() + 1 { ask() -> resume('s') } }",
                "int が必要だが String",
            ),
            (
                "fn main() { handle fail('a') { fail(m) -> resume() } }",
                "never の操作は resume できない",
            ),
            (
                "fn main() { let x = handle 1 { ask() -> resume(2) } }",
                "効果 Ask を起こしていない",
            ),
            (
                "fn f() ! Log { let g = fn() { log('a') } }\nfn main() {}",
                "! Log を書くか",
            ),
            (
                "fn f() ! Log { let g = log }\nfn main() {}",
                "値として使えない",
            ),
            (
                "fn twice(f: (int) -> int, x: int) -> int { f(f(x)) }\nfn n(x: int) -> int ! Log { x }\nfn main() { handle twice(n, 1) { log(m) -> resume() } }",
                "(int) -> int が必要だが (int) -> int ! Log",
            ),
            ("fn g() -> never { 1 }\nfn main() {}", "戻らない式で終わる"),
            ("fn g() -> never { return panic('x') }\nfn main() {}", "return を使えない"),
            // 効果の多相: E は渡した関数の効果になり、呼ぶ側で処理する必要がある
            (
                "fn ap<E: Effect>(f: () -> unit ! E) ! E { f() }\nfn n() ! Log { log('a') }\nfn main() { ap(n) }",
                "! Log を書くか",
            ),
            (
                "fn ap<E: Effect>(f: () -> unit ! E) ! E { log('a') }\nfn main() {}",
                "! Log を書くか",
            ),
            (
                "fn ap<E: Effect>(f: (int) -> unit ! E) ! E { f(1) }\nfn main() { ap(fn(x) { log('a') }) }",
                "! Log を書くか",
            ),
            (
                "fn f() -> int { handle { return 1 } { ask() -> resume(2) } }\nfn main() {}",
                "return を使えない",
            ),
        ] {
            let e = out(&format!("{p}{src}")).unwrap_err();
            assert!(e.contains(msg), "{src} => {e}");
        }
        // 腕の中の操作は外側の handle が処理し、resume しない腕は handle の値になる
        let src = format!(
            "{p}fn main() {{\n let x = handle {{ handle {{ fail('in')\n 1 }} {{ fail(m) -> {{ log(m)\n 2 }} }} }} {{ log(m) -> {{ put('log ' + m)\n resume() }} }}\n put(x) }}"
        );
        assert_eq!(out(&src).unwrap(), "log in\n2\n");
    }

    // トレイト: 既定の実装・条件付きの実装・制約・親のトレイト・== と to_str と put
    #[test]
    fn traits() {
        let o = out(include_str!("../../tests/programs/types/trait.hb")).unwrap();
        assert_eq!(
            o,
            "int 1\nint 2!\n(1, 2)\nPOINT\n[int 1;int 2;]\n[(3, 4);] [(3, 4);]\ntrue\ntrue\nP5,0\n7\nto_string: P1,2\ntrue\n"
        );
        // == は Eq の実装を使う（x だけ比べる実装なら y が違っても等しい）
        let src = "use std -> *\nstruct P { x: int, y: int }\nimpl Eq for P {\n fn eq(self, other: P) -> bool { self.x == other.x }\n}\nfn main() { put(P { x: 1, y: 2 } == P { x: 1, y: 3 }) }";
        assert_eq!(out(src).unwrap(), "true\n");
        let e = out("use std -> *\nfn main() { put(max('a', 'b')) }").unwrap_err();
        assert!(e.msg.contains("Ord<String> の実装が無い"), "{}", e.msg);
        let e = out("use std -> *\nimpl Display for int {\n fn shw(self) -> String { '' }\n}\nfn main() {}").unwrap_err();
        assert!(e.msg.contains("メソッド shw は無い"), "{}", e.msg);
    }

    // struct と enum が実行時の値の形、type は別名と型の計算（実行時に消える）
    #[test]
    fn struct_enum_type() {
        let e = out("type P = { x: int }\nfn main() {}").unwrap_err();
        assert!(e.msg.contains("構造体は struct"), "{}", e.msg);
        let e = out("type S = A | B\nfn main() {}").unwrap_err();
        assert!(e.msg.contains("直和型は enum"), "{}", e.msg);
        let src = "enum S {\n    A\n    B(n: int)\n}\nstruct W(int, String)\nfn main() {\n put(B(1))\n let w = W(1, 'a')\n put(w.1)\n put(w)\n}";
        assert_eq!(out(src).unwrap(), "B(1)\na\nW(1, \"a\")\n");
    }

    #[test]
    fn test_command() {
        let dir = std::env::temp_dir().join(format!("hibana-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (ok, ng) = (dir.join("ok.hb"), dir.join("ng.hb"));
        std::fs::write(&ok, "use std -> *\ntest 'a' {\n let test = 1\n assert_eq(test + 1, 2)\n}\n").unwrap();
        std::fs::write(&ng, "use std -> *\ntest 'b' { assert(false) }\nfn main() { put(1) }\n").unwrap();
        assert_eq!(super::test_cmd(&[ok.to_str().unwrap()]), Ok(true));
        assert_eq!(super::test_cmd(&[ok.to_str().unwrap(), ng.to_str().unwrap()]), Ok(false));
        // it ごとに 1 件。前に書いた文は、それより後の it の前置きになる。it は変数名にも使える
        let nested = dir.join("nested.hb");
        let src = "use std -> *\ntest 'a' {\n let it = 1\n describe 'b' {\n  let y = it + 1\n  it 'c' { assert_eq(y, 2) }\n  it 'd' { assert(y > it) }\n }\n it 'e' { assert(it == 1) }\n}\n";
        std::fs::write(&nested, src).unwrap();
        assert_eq!(super::test_cmd(&[nested.to_str().unwrap()]), Ok(true));
        let labels: Vec<String> = crate::parser::parse(src).unwrap().tests.into_iter().map(|t| t.0).collect();
        assert_eq!(labels, ["a / b / c", "a / b / d", "a / e"]);
        let e = out("test 'a' {\n describe 'b' { let y = 1 }\n}\nfn main() {}").unwrap_err();
        assert!(e.msg.contains("describe 'b' に it が無い"), "{e}");
        let e = out("test 'a' {\n it 'b' { }\n let y = 1\n}\nfn main() {}").unwrap_err();
        assert!(e.msg.contains("どの it でも実行されない"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 型エラーの提案と、効果を起こした操作の位置
    #[test]
    fn error_hints() {
        let e = out("fn main() {\n let count = 1\n put(cuont)\n}").unwrap_err();
        assert_eq!(e.help.as_deref(), Some("count のことか"));
        let e = out("fn main() { prnt(1) }").unwrap_err();
        assert_eq!(e.help.as_deref(), Some("put のことか"));
        let src = "effect Log {\n fn log(m: String)\n}\nfn n() ! Log { log('a') }\nfn main() { n() }";
        let e = out(src).unwrap_err();
        assert_eq!(e.help.as_deref(), Some("関数 n の型が ! Log を持つ"));
        let (at, n) = e.origin.unwrap();
        assert_eq!(&src[at..at + n], "log('a')");
    }

    // 重複した名前のエラーは、後の宣言の名前を指す
    #[test]
    fn duplicate_name_position() {
        let src = "fn f() {}\nstruct f { x: int }\nfn main() {}";
        let e = out(src).unwrap_err();
        assert_eq!(e.span, Some((17, 1)), "{e:?}");
    }

    // 型エラーのメッセージには、誤った式の行を「行 N: 」で前に付ける（式は 3 行目から始まる）
    #[test]
    fn type_error_has_line() {
        let src = "fn main() -> int {\n    let y = 1\n    let z: int = y +\n        's'\n    z\n}\n";
        let e = out(src).unwrap_err();
        assert!(e.msg.starts_with("行 3: 関数 main: "), "{e:?}");
    }

    // match の腕の型エラーは、腕のパターンの行を指す（Circle(a, b) は 8 行目）
    #[test]
    fn arm_error_has_line() {
        let src = "enum Shape {\n    Circle(r: int),\n    Dot,\n}\nfn main() -> int {\n    let s = Circle(1)\n    match s {\n        Circle(a, b) -> 1,\n        Dot -> 0,\n    }\n}\n";
        let e = out(src).unwrap_err();
        assert!(e.msg.starts_with("行 8: "), "{e:?}");
    }

    // data の宣言の型エラーは、宣言の行を指す（struct P は 2 行目。フィールドの式には位置が無い）
    #[test]
    fn data_error_has_line() {
        let src = "fn main() {}\nstruct P {\n    f: Missing,\n}\n";
        let e = out(src).unwrap_err();
        assert!(e.msg.starts_with("行 3: 構築子 P{}: "), "{e:?}");
    }

    // trait の宣言の型エラーは、trait の行を指す（trait Show は 2 行目）
    #[test]
    fn trait_error_has_line() {
        let src = "fn main() {}\ntrait Show {\n    fn show(f: Missing) -> int\n}\n";
        let e = out(src).unwrap_err();
        assert!(e.msg.starts_with("行 3: "), "{e:?}");
    }

    // impl の型エラーは、impl の行を指す（生成した構築子の引数の検査。impl は 5 行目）
    #[test]
    fn impl_error_has_line() {
        let src = "fn main() {}\ntrait Show {\n    fn show(self) -> int\n}\nimpl Show for int {\n    fn show(self) -> String { 'a' }\n}\n";
        let e = out(src).unwrap_err();
        assert!(e.msg.starts_with("行 5: 関数 Show#impl0: "), "{e:?}");
    }

    // 実行時エラーの位置。インタプリタはエラーに位置を付け、JS はメッセージに行と列を足す
    #[test]
    fn runtime_error_positions() {
        let src = "fn half(x: int, y: int) -> int {\n    x / y\n}\nfn main() {\n    put(half(1, 0))\n}\n";
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
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    // よくある条件（routes の添字など）は Z3 なしで証明できる
    #[test]
    fn builtin_prover() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs");
        crate::parser::set_base_dir(&dir);
        for p in hb_files(&dir) {
            crate::parser::set_base_dir(p.parent().unwrap());
            let name = p.file_name().unwrap().to_str().unwrap().to_string();
            // smt.hb は Z3 の例、struct.hb は掛け算の条件を Z3 で示す
            if !name.ends_with(".hb") || name == "smt.hb" || name == "struct.hb" {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap();
            crate::smt::without_z3(|| {
                if name == "wasm_web.hb" {
                    super::to_web(&src, &["echo".into(), "answer".into(), "greet".into()]).map(|_| ())
                } else {
                    super::to_js(&src).map(|_| ())
                }
            }).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    // mut の Vec も、初期値と push・set_at から長さが分かる
    #[test]
    fn mutable_vec_length() {
        assert_eq!(out("fn main() { let mut v = [1, 2]\n v = push(v, 3)\n v = set_at(v, 0, 5)\n put(v[2]) }").unwrap(), "3\n");
        let e = out("fn main() { let mut v = [1, 2]\n v = push(v, 3)\n put(v[3]) }").unwrap_err();
        assert!(e.contains("添字の範囲"), "{e}");
        // ループの後は長さが分からない
        let e = out("fn main() { let mut v = [1]\n for i in 0..2 { v = push(v, 1) }\n put(v[1]) }").unwrap_err();
        assert!(e.contains("添字の範囲"), "{e}");
    }

    #[test]
    fn errors() {
        for (src, msg) in [
            ("fn main() { let x = 1; x = 2; }", "mut でない"),
            ("fn main() { let x: int = true; }", "int が必要"),
            ("fn f() -> int { true } fn main() {}", "戻り値"),
            ("fn main() { if 1 { } }", "if の条件"),
            ("fn main() { put(1 / 0); }", "0 で除算"),
            ("fn main() { y; }", "未定義の変数"),
            ("fn main() { 1 + ; }", "式が必要"),
            ("fn main() { let x = 1 put(x) }", "文の区切り"),
            ("fn main() { let x = quote { 1 } }", "マクロ本体の中でだけ"),
        ] {
            let e = out(src).unwrap_err();
            assert!(e.contains(msg), "{src} => {e}");
        }
    }
}
