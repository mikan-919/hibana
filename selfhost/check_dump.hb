// 引数のファイルごとに、型検査が通れば ok、通らなければ error と最初のエラーを出す（Rust 版との照合用）
use "./io.ts"
use "./checker.hb".*
use "./parser.hb".*
use std.*

fn main() {
    for p in io.args() {
        let errs = check(parse(io.readFile(p)))
        print(if len(errs) > 0 { "error " + errs[0] } else { "ok" })
    }
}
