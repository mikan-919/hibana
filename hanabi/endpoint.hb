use std -> *
use './types.hb' -> *
use './json.hb' -> { Codec, required, decode, decode_path }

struct Fault { status: Status, message: String }
trait HttpError {
    fn status(self) -> Status
    fn invalid(message: String) -> Self
    fn unavailable(message: String) -> Self
}
impl HttpError for Fault {
    fn status(self) -> Status { self.status }
    fn invalid(message: String) -> Fault { Fault { status: 422, message: message } }
    fn unavailable(message: String) -> Fault { Fault { status: 503, message: message } }
}
impl Codec for Fault {
    fn encode(self) -> Json { JObj([Field { key: 'status', value: JInt(self.status) }, Field { key: 'message', value: JStr(self.message) }]) }
    fn decode(value: Json) -> Result<Fault, String> {
        let status = refine<Status>(decode<int>(required(value, 'status')?)?)?
        Ok(Fault { status: status, message: decode<String>(required(value, 'message')?)? })
    }
}
struct Request { method: Method, url: String, body: String }
struct Reply { status: Status, body: String }
struct Endpoint<P, I, O, E> {
    method: Method, pattern: String, success: Status,
    params: (Json) -> Result<P, String>, input: (Json) -> Result<I, String>,
    output: (Json) -> Result<O, String>, failure: (Json) -> Result<E, String>,
    encode_params: (P) -> Json, encode_input: (I) -> Json,
    encode_output: (O) -> Json, encode_failure: (E) -> Json,
    invalid: (String) -> E, unavailable: (String) -> E, error_status: (E) -> Status
}
fn define<P: Codec, I: Codec, O: Codec, E: Codec + HttpError>(method: Method, pattern: String, success: Status) -> Endpoint<P, I, O, E> {
    Endpoint {
        method: method, pattern: pattern, success: success,
        params: fn(v) { decode_path<P>(v) }, input: fn(v) { decode<I>(v) },
        output: fn(v) { decode<O>(v) }, failure: fn(v) { decode<E>(v) },
        encode_params: fn(v) { v.encode() }, encode_input: fn(v) { v.encode() },
        encode_output: fn(v) { v.encode() }, encode_failure: fn(v) { v.encode() },
        invalid: fn(message) { invalid<E>(message) }, unavailable: fn(message) { unavailable<E>(message) }, error_status: fn(e) { e.status() }
    }
}
fn parse_value(text: String) -> Result<Json, String> { match parse_json(text) { Some(v) -> Ok(v) None -> Err('JSON を読めない') } }
fn response<O, P, I, E>(api: Endpoint<P, I, O, E>, reply: Reply) -> Result<O, E> {
    let value = match parse_value(reply.body) { Ok(v) -> v Err(e) -> return Err((api.unavailable)(e)) }
    if reply.status == api.success {
        match (api.output)(value) { Ok(v) -> Ok(v) Err(e) -> Err((api.unavailable)(e)) }
    } else {
        match (api.failure)(value) { Ok(e) -> Err(e) Err(e) -> Err((api.unavailable)(e)) }
    }
}
// 一つの引数はスカラー、複数の引数は record で宣言する。
fn parameters(pattern: String, path: String) -> Option<Json> {
    let template = split(pattern, '/')
    let actual = split(path, '/')
    if len(template) != len(actual) { return None }
    let mut values: Vec<Field> = []
    for i in 0..len(template) {
        let piece = template[i]
        let raw = if i < len(actual) { actual[i] } else { return None }
        if starts_with(piece, '{') && ends_with(piece, '}') {
            values = push(values, Field { key: slice(piece, 1, len(piece) - 1), value: JStr(raw) })
        } else if piece != raw { return None }
    }
    if len(values) == 0 { Some(JNull) }
    else if len(values) == 1 { Some(values[0].value) }
    else { Some(JObj(values)) }
}
fn request<P, I, O, E>(api: Endpoint<P, I, O, E>, params: P, input: I) -> Request {
    let value = (api.encode_params)(params)
    let mut parts: Vec<String> = []
    for piece in split(api.pattern, '/') {
        if starts_with(piece, '{') && ends_with(piece, '}') {
            let key = slice(piece, 1, len(piece) - 1)
            let part = match value { JObj(_) -> match field(value, key) { Some(v) -> v None -> JNull } _ -> value }
            let raw = match part { JStr(s) -> s JInt(n) -> n.to_string() _ -> panic('経路引数は文字列か整数が必要') }
            if includes(raw, '/') || includes(raw, '?') || includes(raw, '#') || includes(raw, '%') { panic('経路引数に区切り文字は使えない') }
            parts = push(parts, raw)
        } else { parts = push(parts, piece) }
    }
    Request { method: api.method, url: join(parts, '/'), body: stringify((api.encode_input)(input)) }
}
syntax ApiEntry = name:Ident method:Ident path:String '(' params:Type ',' input:Type ')' '->' output:Type '!' failure:Type '=' success:Int
macro Api('api' '{' entries:ApiEntry* '}') -> Decl {
    let mut declarations = quote {}
    let mut seen: Vec<String> = []
    let mut index = 0
    while index < len(entries) {
        let entry = at(entries, index)
        index = index + 1
        let method = str_of(entry.method)
        let verb = if method == 'GET' { quote { Get } } else if method == 'POST' { quote { Post } } else if method == 'PUT' { quote { Put } } else if method == 'DELETE' { quote { Delete } } else { error('HTTP メソッドが未対応: ' + method) }
        let path = str_of(entry.path)
        if !starts_with(path, '/') { error('経路は / で始める') }
        let mut captures: Vec<String> = []
        let mut shape = ''
        for piece in split(path, '/') {
            if starts_with(piece, '{') || (len(piece) > 0 && slice(piece, len(piece) - 1, len(piece)) == '}') {
                if !starts_with(piece, '{') || !(len(piece) > 0 && slice(piece, len(piece) - 1, len(piece)) == '}') || len(piece) <= 2 { error('経路引数は {名前} と書く') }
                let key = slice(piece, 1, len(piece) - 1)
                for previous in captures { if previous == key { error('経路引数が重複している: ' + key) } }
                captures = push(captures, key)
                shape = shape + '/{}'
            } else { shape = shape + '/' + piece }
        }
        if (len(captures) == 0) != (str_of(entry.params) == 'unit') { error('経路引数が無い宣言の型は unit にする') }
        let signature = method + shape
        for previous in seen { if previous == signature { error('経路が重複している: ' + signature) } }
        seen = push(seen, signature)
        declarations = quote {
            $declarations
            fn $entry.name() -> Endpoint<$entry.params, $entry.input, $entry.output, $entry.failure> {
                define<$entry.params, $entry.input, $entry.output, $entry.failure>($verb, $entry.path, $entry.success)
            }
        }
    }
    declarations
}
