use std -> *
use './types.hb' -> *
use './endpoint.hb'
use './server.hb'

fn route<P, I, O, E>(request: server.Request, api: endpoint.Endpoint<P, I, O, E>, handler: (P, I) -> Result<O, E>) -> Option<server.Response> {
    if request.method != api.method { return None }
    let path = match endpoint.parameters(api.pattern, request.path) { Some(v) -> v None -> return None }
    let decoded = (api.params)(path)
    let result: Result<O, E> = match decoded {
        Err(message) -> Err((api.invalid)(message))
        Ok(params) -> {
            let input: Result<Json, String> = if api.method == Get { Ok(JNull) }
            else if !request.json { Err('JSON 要求が必要') }
            else if !server.valid_body(request.body) { Err('文字コードが UTF-8 でない') }
            else { endpoint.parse_value(server.body_text(request.body)) }
            match input {
                Err(message) -> Err((api.invalid)(message))
                Ok(value) -> match (api.input)(value) { Err(message) -> Err((api.invalid)(message)) Ok(value) -> handler(params, value) }
            }
        }
    }
    Some(match result {
        Ok(value) -> server.json(api.success, (api.encode_output)(value))
        Err(error) -> server.json((api.error_status)(error), (api.encode_failure)(error))
    })
}
syntax RouteBinding = api:Ident '->' handler:Expr
macro Routes('routes' request:Expr '{' entries:(RouteBinding % ';') '}') {
    let mut result = quote { server.error(404, '経路が見つからない') }
    let mut n = len(entries)
    while n > 0 {
        n = n - 1
        let entry = at(entries, n)
        let mut j = 0
        while j < n {
            if str_of(at(entries, j).api) == str_of(entry.api) { error('経路の登録が重複している: ' + str_of(entry.api)) }
            j = j + 1
        }
        result = quote { match route(incoming, $entry.api(), $entry.handler) { Some(response) -> response None -> $result } }
    }
    quote { { let incoming = $request; $result } }
}
