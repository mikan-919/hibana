// libz3 は動的にリンクする。nix develop の外でも実行時に見つけられるよう、ライブラリのディレクトリ（Z3_LIB_DIR、
// flake.nix が設定する）を rpath に入れる
// ponytail: Nix のストアの経路を埋め込むので、そのストアが消えると動かない。配布するなら --features static で Z3 ごと静的にリンクする
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=Z3_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_STATIC").is_some() {
        return;
    }
    if let Ok(dir) = std::env::var("Z3_LIB_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
