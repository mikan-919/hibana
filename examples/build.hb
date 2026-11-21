// first.hb を JavaScript とネイティブにビルドする。`hibana examples/build.hb` で実行する
// 経路はこのファイルのディレクトリから数える
use std::build
use std::fs

fn main() {
    let src = 'first.hb'
    fs::write_bytes('dist/first.mjs', build::compile(src, 'js'))
    put('js -> dist/first.mjs')
    build::write_executable('dist/first', build::compile(src, 'native'))
    put('native -> dist/first')
}
