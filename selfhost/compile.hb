// セルフホストのコンパイラー。引数のファイルと、それが use するモジュールを検査して、JavaScript を標準出力に出す
use "./io.ts"
use "./checker.hb".*
use "./codegen.hb".*
use "./loader.hb".*
use std.*

fn main() {
    for p in io.args() {
        let prog = load(p)
        let errs = check(prog)
        if len(errs) > 0 { for e in errs { print("// エラー: " + e) } } else { print(compile(prog)) }
    }
}
