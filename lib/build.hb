// ビルドスクリプト用の関数。use std::build で読み込む。インタプリタでの実行（hibana <ファイル>）でだけ使える
// 経路は、実行したファイルのディレクトリから数える
use std -> { Pair }

// path のファイルを target（'js'・'wasm'・'native'）にコンパイルしたバイト列
pub fn compile(path: String, target: String) -> Vec<int> with {} { compile_file(path, target) }

// exports の関数を公開する WASM ライブラリのバイト列と、受け渡し規約の JSON
pub fn compile_wasm(path: String, exports: Vec<String>) -> Pair<Vec<int>, String> with {} { compile_wasm_file(path, exports) }

// 実行権限を付けてファイルに書く。親ディレクトリが無ければ作る
pub fn write_executable(path: String, bytes: Vec<int>) with {} { write_file(path, bytes, true) }
