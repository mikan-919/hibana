// ビルド時の通常の関数。Web 固有の生成はコンパイラではなくこのライブラリが持つ。
fn build_text(path: str, text: str) {
    artifact(path, utf8(text))
}

fn build_web(wasm_path: str, js_path: str, source: str, exports: Vec<str>) {
    let wasm = compile(source, 'wasm', exports)
    let javascript = web_javascript(inspect_exports(source, exports))
    artifact(wasm_path, wasm)
    build_text(js_path, javascript)
}

fn web_javascript(metadata: str) -> str { js_glue(metadata) }

macro web_build ('web_build' wasm:String ',' glue:String 'source' source:String 'exports' '(' names:(Ident % ',') ')') -> Decl {
    let mut exports: Vec<str> = []
    let mut i = 0
    while i < len(names) {
        exports = push(exports, str_of(at(names, i)))
        i = i + 1
    }
    build_web(str_of(wasm), str_of(glue), str_of(source), exports)
    quote {}
}

macro build_asset ('build_asset' path:String ',' contents:String) -> Decl {
    build_text(str_of(path), str_of(contents))
    quote {}
}
