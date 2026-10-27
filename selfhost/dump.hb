// 引数のファイルごとに、トークンを「行:列 トークン」で 1 行ずつ出す（Rust 版との照合用）
use "./io.ts"
use "./lexer.hb".*
use std.*

fn main() {
    for p in io.args() {
        match lex(io.readFile(p)) {
            Ok(ts) => for t in ts { print(to_str(t.line) + ":" + to_str(t.col) + " " + to_str(t.tok)) },
            Err(e) => print(p + ":" + to_str(e.line) + ":" + to_str(e.col) + " " + e.msg),
        }
    }
}
