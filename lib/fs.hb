// ファイルの書き込み。use std::fs で読み込む。インタプリタでの実行（hibana <ファイル>）でだけ使える
// 経路は、実行したファイルのディレクトリから数える

// ファイルに書く。親ディレクトリが無ければ作る。各要素は 0〜255
pub fn write_bytes(path: String, bytes: Vec<int>) with {} { write_file(path, bytes, false) }
