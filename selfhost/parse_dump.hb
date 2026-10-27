// 引数のファイルごとに、構文木を S 式で出す（Rust 版との照合用）
use "./io.ts"
use "./parser.hb".*
use std.*

fn main() {
    for p in io.args() { print(program(parse(io.readFile(p)))) }
}
