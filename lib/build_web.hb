// ビルド時の通常の関数。Web 固有の生成はコンパイラではなくこのライブラリが持つ。
fn build_text(path: str, text: str) {
    artifact(path, utf8(text))
}

fn build_web(wasm_path: str, js_path: str, source: str, exports: Vec<str>) {
    let wasm = compile(source, "wasm", exports)
    let javascript = web_javascript(inspect_exports(source, exports))
    artifact(wasm_path, wasm)
    build_text(js_path, javascript)
}

fn web_javascript(metadata: str) -> str {
    "const metadata = " + metadata + ";\n// hibana が生成した JavaScript 接続コード\nexport async function instantiate(bytes, imports = {}) {\n  let instance;\n  let active = false;\n  const encoder = new TextEncoder();\n  const decoder = new TextDecoder(\"utf-8\", { fatal: true });\n  function encode(value, type) {\n    if (type === \"unit\") {\n      if (value !== undefined) throw new TypeError(\"unit の戻り値は undefined が必要\");\n      return undefined;\n    }\n    if (type === \"str\") {\n      if (typeof value !== \"string\") throw new TypeError(\"文字列が必要\");\n      const data = encoder.encode(value);\n      const ptr = instance.exports.__hibana_alloc(data.length) >>> 0;\n      new Uint8Array(instance.exports.memory.buffer, ptr, data.length).set(data);\n      return BigInt.asIntN(64, (BigInt(data.length) << 32n) | BigInt(ptr));\n    }\n    if (typeof value !== \"number\" && typeof value !== \"bigint\") throw new TypeError(\"整数が必要\");\n    if (typeof value === \"number\" && !Number.isSafeInteger(value)) throw new RangeError(\"安全な整数が必要\");\n    const n = BigInt(value);\n    if (n < -(1n << 63n) || n >= (1n << 63n)) throw new RangeError(\"64 ビット整数の範囲外\");\n    return n;\n  }\n  function decode(value, type) {\n    if (type === \"unit\") return undefined;\n    if (type === \"int\") return value;\n    const packed = BigInt.asUintN(64, value);\n    const ptr = Number(packed & 0xffffffffn);\n    const len = Number(packed >> 32n);\n    return decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len));\n  }\n  const host = Object.create(null);\n  for (const [name, params, result] of metadata.imports) {\n    const callback = imports.hibana_host?.[name];\n    if (typeof callback !== \"function\") throw new TypeError(\"外部関数が必要: \" + name);\n    host[name] = (...args) => encode(callback(...args.map((value, i) => decode(value, params[i]))), result);\n  }\n  const instantiated = await WebAssembly.instantiate(bytes, { ...imports, hibana_host: host });\n  instance = instantiated instanceof WebAssembly.Instance ? instantiated : instantiated.instance;\n  const functions = Object.create(null);\n  for (const [name, params, result] of metadata.exports) {\n    functions[name] = (...args) => {\n      if (active) throw new Error(\"同じ WASM インスタンスへ再入できない\");\n      if (args.length !== params.length) throw new TypeError(\"引数の数が違う\");\n      active = true;\n      try {\n        instance.exports.__hibana_reset?.();\n        const value = instance.exports[name](...args.map((value, i) => encode(value, params[i])));\n        return decode(value, result);\n      } finally {\n        instance.exports.__hibana_reset?.();\n        active = false;\n      }\n    };\n  }\n  return functions;\n}\n\n"
}

macro web_build ("web_build" wasm:String "," glue:String "source" source:String "exports" "(" names:(Ident % ",") ")") -> Decl {
    let mut exports: Vec<str> = []
    let mut i = 0
    while i < len(names) {
        exports = push(exports, str_of(at(names, i)))
        i = i + 1
    }
    build_web(str_of(wasm), str_of(glue), str_of(source), exports)
    quote {}
}

macro build_asset ("build_asset" path:String "," contents:String) -> Decl {
    build_text(str_of(path), str_of(contents))
    quote {}
}
