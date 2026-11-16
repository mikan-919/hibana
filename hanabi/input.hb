use std -> *
use './server.hb'
use './native.hb'

effect Rejected { fn reject(field: String, message: String) -> never }
trait Decode { fn decode(value: Json) -> Self ! Rejected }

fn text(value: Json, name: String) -> String ! Rejected {
    match field(value, name) {
        Some(v) -> match v { JStr(s) -> s _ -> reject(name, '文字列が必要') }
        None -> reject(name, '入力が必要')
    }
}
fn integer(value: Json, name: String) -> int ! Rejected {
    match field(value, name) {
        Some(v) -> match v { JInt(n) -> n _ -> reject(name, '整数が必要') }
        None -> reject(name, '入力が必要')
    }
}
fn json_body<A: Decode>(request: server.Request) -> A ! Rejected {
    if !request.json { reject('request', 'hanabi の JSON 要求が必要') }
    if !native.valid_utf8(request.body) { reject('request', '文字コードが UTF-8 でない') }
    match parse_json(native.decode(request.body)) {
        Some(value) -> decode(value)
        None -> reject('request', 'JSON を読めない')
    }
}
