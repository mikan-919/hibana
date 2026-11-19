use std -> *

// コンパイラーが返した受け渡し規約を使う JavaScript モジュールを作る。
pub fn javascript(abi: String) -> String with {} {
    'const abi = ' + abi + ';\n' + 'export async function instantiate(bytes, imports = {}) {
  let instance;
  let active = false;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
  // Vec・構造体・Result は 8 バイトの語の並びで渡す（runtime.hb の __val_in・__val_out）。
  // 値は [型の番号][中身]、文字列は [3][長さ][バイト列]、Vec は [4][要素数][要素...]、構造体と Result は [5][構築子][フィールド数][フィールド...]
  function put(out, type, value) {
    if (typeof type === "object" && type.vec !== undefined) {
      if (!Array.isArray(value)) throw new TypeError("配列が必要");
      out.push(4n, BigInt(value.length));
      for (const item of value) put(out, type.vec, item);
    } else if (typeof type === "object") {
      if (typeof value !== "object" || value === null) throw new TypeError("オブジェクトが必要");
      out.push(5n, BigInt(type.ctor), BigInt(type.fields.length));
      for (const [name, field] of type.fields) {
        if (!(name in value)) throw new TypeError("フィールドが必要: " + name);
        put(out, field, value[name]);
      }
    } else if (type === "String") {
      if (typeof value !== "string") throw new TypeError("文字列が必要");
      const data = encoder.encode(value);
      out.push(3n, BigInt(data.length));
      for (let i = 0; i < data.length; i += 8) {
        let word = 0n;
        for (let k = 7; k >= 0; k--) word = (word << 8n) | BigInt(data[i + k] ?? 0);
        out.push(BigInt.asIntN(64, word));
      }
    } else if (type === "number") {
      if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError("有限な number が必要");
      out.push(9n, new BigInt64Array(new Float64Array([value]).buffer)[0]);
    } else if (type === "bool") {
      if (typeof value !== "boolean") throw new TypeError("真偽値が必要");
      out.push(2n, value ? 1n : 0n);
    } else if (type === "unit") {
      if (value !== undefined) throw new TypeError("unit は undefined が必要");
      out.push(0n);
    } else {
      if (typeof value !== "number" || !Number.isSafeInteger(value)) throw new RangeError("安全な整数が必要");
      out.push(1n, BigInt(value));
    }
  }
  function take(view, at, type) {
    const word = (k) => view.getBigInt64(at + 8 * k, true);
    if (typeof type === "object" && type.result !== undefined) {
      const [[okCtor, okType], [errCtor, errType]] = type.result;
      const ctor = word(1);
      if (word(0) !== 5n || word(2) !== 1n) throw new TypeError("Result の形式が不正");
      if (ctor === BigInt(okCtor)) {
        const [value, next] = take(view, at + 24, okType);
        return [{ ok: true, value }, next];
      }
      if (ctor === BigInt(errCtor)) {
        const [error, next] = take(view, at + 24, errType);
        return [{ ok: false, error }, next];
      }
      throw new TypeError("Result の構築子が不正");
    }
    if (typeof type === "object" && type.vec !== undefined) {
      const items = [];
      let next = at + 16;
      for (let i = 0; i < Number(word(1)); i++) {
        const [item, after] = take(view, next, type.vec);
        items.push(item);
        next = after;
      }
      return [items, next];
    }
    if (typeof type === "object") {
      const object = {};
      let next = at + 24;
      for (const [name, field] of type.fields) {
        const [item, after] = take(view, next, field);
        object[name] = item;
        next = after;
      }
      return [object, next];
    }
    if (type === "String") {
      const n = Number(word(1));
      return [decoder.decode(new Uint8Array(view.buffer, view.byteOffset + at + 16, n)), at + 16 + Math.ceil(n / 8) * 8];
    }
    if (type === "unit") return [undefined, at + 8];
    if (type === "bool") return [word(1) !== 0n, at + 16];
    if (word(0) === 9n) return [view.getFloat64(at + 8, true), at + 16];
    const n = Number(word(1));
    return [n === 0 ? 0 : n, at + 16];
  }
  function encode(value, type) {
    if (typeof type === "object") {
      const words = [];
      put(words, type, value);
      const data = new Uint8Array(new BigInt64Array(words).buffer);
      const ptr = instance.exports.__hibana_alloc(data.length) >>> 0;
      new Uint8Array(instance.exports.memory.buffer, ptr, data.length).set(data);
      return BigInt.asIntN(64, (BigInt(data.length) << 32n) | BigInt(ptr));
    }
    if (type === \'bool\') {
      if (typeof value !== \'boolean\') throw new TypeError(\'真偽値が必要\');
      return value ? 1n : 0n;
    }
    if (type === "unit") {
      if (value !== undefined) throw new TypeError("unit の戻り値は undefined が必要");
      return undefined;
    }
    if (type === "String") {
      if (typeof value !== "string") throw new TypeError("文字列が必要");
      const data = encoder.encode(value);
      const ptr = instance.exports.__hibana_alloc(data.length) >>> 0;
      new Uint8Array(instance.exports.memory.buffer, ptr, data.length).set(data);
      return BigInt.asIntN(64, (BigInt(data.length) << 32n) | BigInt(ptr));
    }
    if (type === "number") {
      if (typeof value !== "number" || !Number.isFinite(value)) throw new TypeError("有限な number が必要");
      return value;
    }
    if (typeof value !== "number" || !Number.isSafeInteger(value)) throw new RangeError("安全な整数が必要");
    return value === 0 ? 0 : value;
  }
  function decode(value, type) {
    if (typeof type === "object") {
      const packed = BigInt.asUintN(64, value);
      const view = new DataView(instance.exports.memory.buffer, Number(packed & 0xffffffffn), Number(packed >> 32n));
      return take(view, 0, type)[0];
    }
    if (type === \'bool\') return value !== 0n;
    if (type === "unit") return undefined;
    if (type === "int") {
      if (typeof value !== "number" || !Number.isSafeInteger(value)) throw new RangeError("安全な整数が必要");
      return value === 0 ? 0 : value;
    }
    if (type === "number") {
      if (!Number.isFinite(value)) throw new RangeError("number の演算結果が有限でない");
      return value;
    }
    const packed = BigInt.asUintN(64, value);
    const ptr = Number(packed & 0xffffffffn);
    const len = Number(packed >> 32n);
    return decoder.decode(new Uint8Array(instance.exports.memory.buffer, ptr, len));
  }
  const host = Object.create(null);
  const hostSignatures = abi.imports;
  for (const [name, params, result] of hostSignatures) {
    const callback = imports.hibana_host?.[name];
    if (typeof callback !== "function") throw new TypeError("外部関数が必要: " + name);
    host[name] = (...args) => encode(callback(...args.map((value, i) => decode(value, params[i]))), result);
  }
  // 標準出力と標準エラーは別の関数に渡す
  const print = imports.hibana?.print ?? console.log;
  const error = imports.hibana?.error ?? console.error;
  const hibana = {
    write: (fd, ptr, len) => {
      const output = Number(fd) === 2 ? error : print;
      output(decoder.decode(new Uint8Array(instance.exports.memory.buffer, Number(ptr), Number(len))).replace(/\\n$/, ""));
    },
    now_ms: () => BigInt(Date.now()),
    read: (fd, ptr, len) => {
      const target = new Uint8Array(instance.exports.memory.buffer, Number(ptr), Number(len));
      if (imports.hibana?.read) return BigInt(imports.hibana.read(Number(fd), target));
      if (typeof process !== "undefined" && process.getBuiltinModule) {
        const fs = process.getBuiltinModule("node:fs");
        try { return BigInt(fs.readSync(Number(fd), target, 0, target.length, null)); }
        catch { return -1n; }
      }
      throw new Error("標準入力には imports.hibana.read が必要です");
    },
  };
  const instantiated = await WebAssembly.instantiate(bytes, { ...imports, hibana_host: host, hibana });
  instance = instantiated instanceof WebAssembly.Instance ? instantiated : instantiated.instance;
  const functions = Object.create(null);
  const exportSignatures = abi.exports;
  for (const [name, params, result] of exportSignatures) {
    functions[name] = (...args) => {
      if (active) throw new Error("同じ WASM インスタンスへ再入できない");
      if (args.length !== params.length) throw new TypeError("引数の数が違う");
      active = true;
      try {
        instance.exports.__hibana_reset?.();
        const value = instance.exports[name](...args.map((value, i) => encode(value, params[i])));
        return decode(value, result);
      } catch (e) {
        if (e instanceof WebAssembly.RuntimeError && instance.exports.memory) {
          const memory = instance.exports.memory.buffer;
          const view = new DataView(memory);
          const failure = abi.failure_offset;
          const ptr = Number(view.getBigUint64(failure, true));
          if (ptr !== 0) {
            view.setBigUint64(failure, 0n, true);
            const len = Number(view.getBigUint64(ptr, true));
            e.message = "実行時エラー: " + decoder.decode(new Uint8Array(memory, ptr + 8, len));
          }
        }
        throw e;
      } finally {
        instance.exports.__hibana_reset?.();
        active = false;
      }
    };
  }
  return functions;
}
'
}

fn array(value: Json) -> Vec<Json> {
    match value {
        JArr(items) -> items
        _ -> panic('WASM の受け渡し規約に配列が必要')
    }
}

fn item(values: Vec<Json>, i: int) -> Json {
    if 0 <= i && i < len(values) { values[i] } else { panic('WASM の受け渡し規約の項目が足りない') }
}

fn text(value: Json) -> String {
    match value {
        JStr(s) -> s
        _ -> panic('WASM の受け渡し規約に文字列が必要')
    }
}

fn ts_type(ty: Json) -> String {
    match ty {
        JStr(name) -> {
            return match name {
                'int' -> 'number'
                'number' -> 'number'
                'bool' -> 'boolean'
                'String' -> 'string'
                'unit' -> 'undefined'
                _ -> panic('WASM の受け渡し規約に未対応の型がある')
            }
        }
        _ -> {}
    }
    match field(ty, 'vec') {
        Some(element) -> { return 'Array<' + ts_type(element) + '>' }
        None -> {}
    }
    match field(ty, 'result') {
        Some(cases) -> {
            let variants = array(cases)
            let ok_case = item(array(item(variants, 0)), 1)
            let err_case = item(array(item(variants, 1)), 1)
            return '{ ok: true; value: ' + ts_type(ok_case) + ' } | { ok: false; error: ' + ts_type(err_case) + ' }'
        }
        None -> {}
    }
    let mut out = '{'
    for entry in array(field(ty, 'fields').unwrap()) {
        let member = array(entry)
        out = out + ' ' + quote_str(text(item(member, 0))) + ': ' + ts_type(item(member, 1)) + ';'
    }
    out + ' }'
}

fn ts_signature(entry: Json) -> String {
    let signature = array(entry)
    let name = quote_str(text(item(signature, 0)))
    let mut params: Vec<String> = []
    let mut i = 0
    for ty in array(item(signature, 1)) {
        params = push(params, 'arg' + i.to_string() + ': ' + ts_type(ty))
        i = i + 1
    }
    name + ': (' + params.join(', ') + ') => ' + ts_type(item(signature, 2)) + ';'
}

// javascript と同じ JavaScript の値の形を TypeScript の型として出す。
pub fn typescript(abi: String) -> String with {} {
    let metadata = parse_json(abi).unwrap()
    let imports = array(field(metadata, 'imports').unwrap())
    let mut out = 'type Imports = WebAssembly.Imports & {
  hibana?: {
    print?: (message: string) => void;
    error?: (message: string) => void;
    read?: (fd: number, target: Uint8Array) => number | bigint;
  };
'
    let mut imports_arg = 'imports?: Imports'
    if len(imports) > 0 {
        out = out + '  hibana_host: {\n'
        for entry in imports { out = out + '    ' + ts_signature(entry) + '\n' }
        out = out + '  };\n'
        imports_arg = 'imports: Imports'
    }
    out = out + '};\n\nexport function instantiate(bytes: BufferSource | WebAssembly.Module, ' + imports_arg + '): Promise<{\n'
    for entry in array(field(metadata, 'exports').unwrap()) {
        out = out + '  ' + ts_signature(entry) + '\n'
    }
    out + '}>;\n'
}

// 生成した文字列を UTF-8 のバイト列に変換する。
pub fn utf8(source: String) -> Vec<int> with {} {
    let mut bytes: Vec<int> = []
    for c in chars(source) {
        let n = code(c)
        if n < 0x80 {
            bytes = push(bytes, n)
        } else {
            if n < 0x800 {
                bytes = push(bytes, 0xC0 + n / 64)
            } else if n < 0x10000 {
                bytes = push(bytes, 0xE0 + n / 4096)
                bytes = push(bytes, 0x80 + n / 64 % 64)
            } else {
                bytes = push(bytes, 0xF0 + n / 262144)
                bytes = push(bytes, 0x80 + n / 4096 % 64)
                bytes = push(bytes, 0x80 + n / 64 % 64)
            }
            bytes = push(bytes, 0x80 + n % 64)
        }
    }
    bytes
}
