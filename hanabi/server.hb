use std -> *
use './native.hb'
use './types.hb' -> *
use './storage.hb' -> { Storage }

struct Request { method: Method, path: String, body: Bytes, json: bool }
struct Response { status: Status, content_type: String, body: Bytes }

fn bytes(status: Status, content_type: String, body: Bytes) -> Response {
    Response { status: status, content_type: content_type, body: body }
}
fn text(status: Status, body: String) -> Response { bytes(status, 'text/plain; charset=utf-8', native.utf8(body)) }
fn html(status: Status, body: String) -> Response { bytes(status, 'text/html; charset=utf-8', native.utf8(body)) }
fn json(status: Status, value: Json) -> Response { bytes(status, 'application/json; charset=utf-8', native.utf8(stringify(value))) }
fn error(status: Status, message: String) -> Response {
    json(status, JObj([Field { key: 'error', value: JStr(message) }]))
}
fn asset(path: FilePath, content_type: String) -> Response
with Storage(storage) {
    match storage.asset(path) {
        Ok(value) -> match value { Some(data) -> bytes(200, content_type, data) None -> error(404, 'ファイルが見つからない') }
        Err(message) -> error(500, message)
    }
}
fn valid_body(bytes: Bytes) -> bool { native.valid_utf8(bytes) }
fn body_text(bytes: Bytes) -> String { native.decode(bytes) }
fn lower_ascii(value: String) -> String {
    let mut result = ''
    for ch in chars(value) {
        let n = code(ch)
        result = result + if 65 <= n && n <= 90 { char_from_codepoint(n + 32) } else { ch }
    }
    result
}

fn header_end(data: Bytes) -> int[n -> n == -1 || (4 <= n && n <= len(data))] {
    for i in 0..len(data) {
        if i + 3 < len(data) {
            if data[i] == 13 && data[i + 1] == 10 && data[i + 2] == 13 && data[i + 3] == 10 { return i + 4 }
        }
    }
    -1
}
fn take(data: Bytes, start: int[n -> 0 <= n && n <= len(data)], end: int[n -> start <= n && n <= len(data)]) -> Bytes {
    native.slice(data, start, end)
}
fn receive(fd: Socket) -> Result<Request, String> {
    let mut data: Bytes = []
    let mut end = -1
    while end < 0 {
        let part = native.read(fd, 4096)?
        if len(part) == 0 { return Err('要求が途中で終わった') }
        data = native.join(data, part)
        end = header_end(data)
        if end > 16384 || (end < 0 && len(data) > 16384) { return Err('ヘッダーの上限を超えた') }
    }
    if end < 0 || end > len(data) { return Err('ヘッダーの境界が不正') }
    let head = take(data, 0, end)
    for b in head { if b > 127 { return Err('ヘッダーは ASCII で書く') } }
    let lines = split(native.decode(head), '\r\n')
    let mut first_line = ''
    for i in 0..len(lines) { if i == 0 { first_line = lines[i] } }
    let mut method = ''
    let mut path = ''
    let mut version = ''
    let mut fields = 0
    for word in split(first_line, ' ') {
        if fields == 0 { method = word }
        else if fields == 1 { path = word }
        else if fields == 2 { version = word }
        fields = fields + 1
    }
    if fields != 3 { return Err('要求行を読めない') }
    let method = parse_method(method)?
    if version != 'HTTP/1.1' && version != 'HTTP/1.0' { return Err('HTTP の版が未対応') }
    let mut size = 0
    let mut seen_size = false
    let mut is_json = false
    let mut hanabi = false
    for line in lines {
        let pair = split(line, ':')
        if len(pair) >= 2 {
            let key = lower_ascii(pair[0])
            let value = trim(slice(line, len(key) + 1, len(line)))
            if key == 'content-length' {
                if seen_size { return Err('Content-Length は一度だけ指定する') }
                seen_size = true
                size = parse_int(value)?
                if size < 0 || size > 131072 { return Err('本文の上限は128KiB') }
            }
            if key == 'transfer-encoding' { return Err('Transfer-Encoding は未対応') }
            if key == 'content-type' { is_json = value == 'application/json' || starts_with(value, 'application/json;') }
            if key == 'x-hanabi-request' { hanabi = value == '1' }
        }
    }
    while len(data) < end + size {
        let part = native.read(fd, 4096)?
        if len(part) == 0 { return Err('本文が途中で終わった') }
        data = native.join(data, part)
    }
    if end < 0 || end + size < end || end + size > len(data) { return Err('本文の境界が不正') }
    Ok(Request { method: method, path: path, body: take(data, end, end + size), json: is_json && hanabi })
}
fn send(fd: Socket, response: Response) -> Result<unit, String> {
    let reason = if response.status < 400 { 'OK' } else { 'Error' }
    let head = 'HTTP/1.1 ' + response.status.to_string() + ' ' + reason + '\r\nContent-Type: ' + response.content_type + '\r\nContent-Length: ' + len(response.body).to_string() + '\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\n\r\n'
    native.write(fd, native.utf8(head))?
    native.write(fd, response.body)
}
fn serve(port: Port, handler: (Request) -> Response) {
    let listener = native.listen(port)
    put('hanabi http://127.0.0.1:' + port.to_string())
    // ponytail: 一つのプロセスで処理する。並行処理には保存の排他制御と要求単位のメモリ回収を足す。
    while true {
        let fd = native.accept(listener)
        let response = match receive(fd) { Ok(req) -> handler(req) Err(e) -> error(400, e) }
        send(fd, response)
        native.close(fd)
    }
}
