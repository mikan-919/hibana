mod eval;
mod lexer;
mod parser;
mod typeck;

use std::io::Write;

fn run(src: &str, out: &mut dyn Write) -> Result<eval::Value, String> {
    let fns = parser::parse(lexer::lex(src)?)?;
    typeck::check(&fns)?;
    eval::run(&fns, out)
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("使い方: hibana <ファイル>");
        std::process::exit(2);
    };
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(2);
    });
    if let Err(e) = run(&src, &mut std::io::stdout()) {
        eprintln!("エラー: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    fn out(src: &str) -> Result<String, String> {
        let mut buf = Vec::new();
        super::run(src, &mut buf)?;
        Ok(String::from_utf8(buf).unwrap())
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
            ("fn main() { let x: int = true; }", "Int が必要"),
            ("fn f() -> int { true } fn main() {}", "戻り値"),
            ("fn main() { if 1 { } }", "if の条件"),
            ("fn main() { print(1 / 0); }", "0 で除算"),
            ("fn main() { y; }", "未定義の変数"),
            ("fn main() { 1 + ; }", "式が必要"),
        ] {
            let e = out(src).unwrap_err();
            assert!(e.contains(msg), "{src} => {e}");
        }
    }
}
