// ビルド時だけ動くライブラリ。use build_web で構文を読み込む。
// 成果物は build コマンドが型検査後に生成する。main は要らない。

macro web_build ("web_build" wasm:String "," glue:String "exports" "(" names:(Ident % ",") ")") -> Decl {
    let mut exports: Vec<str> = []
    let mut i = 0
    while i < len(names) {
        exports = push(exports, str_of(at(names, i)))
        i = i + 1
    }
    build_wasm(str_of(wasm), exports)
    build_glue(str_of(glue), exports)
    quote {}
}

macro build_asset ("build_asset" path:String "," contents:String) -> Decl {
    build_emit(str_of(path), str_of(contents))
    quote {}
}
