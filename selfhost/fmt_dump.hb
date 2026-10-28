// 引数のファイルごとに、整形した結果と区切りの行を出す（Rust 版との照合用）
use "./io.ts"
use "./fmt.hb".*
use std.*

fn main() {
    for p in io.args() {
        match format(io.readFile(p)) {
            Ok(s) => print(s),
            Err(e) => print("ERROR"),
        }
        print("=====")
    }
}
