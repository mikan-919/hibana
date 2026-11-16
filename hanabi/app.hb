use std -> *
use './server.hb' -> { Request, Response }
use './server.hb'
use './json.hb' -> { ToJson }
use './view.hb' -> { Html }
use './native.hb'

type Request = server.Request
type Response = server.Response
fn ok<A: ToJson>(value: A) -> Response { server.json(200, value.to_json()) }
fn created<A: ToJson>(value: A) -> Response { server.json(201, value.to_json()) }
fn page(html: Html) -> Response { server.html(200, html.0) }
fn invalid(field: String, message: String) -> Response {
    server.json(422, object { field: field, error: message })
}
fn not_found(message: String) -> Response { server.error(404, message) }
fn internal_error(message: String) -> Response { server.error(500, message) }
fn run(port: int, handler: (Request) -> Response) {
    native.mkdir('data')
    server.serve(port, fn(request) {
        if request.method == 'GET' {
            match request.path {
                '/' -> server.asset('index.html', 'text/html; charset=utf-8')
                '/style.css' -> server.asset('style.css', 'text/css; charset=utf-8')
                '/browser.mjs' -> server.asset('browser.mjs', 'text/javascript; charset=utf-8')
                '/client.mjs' -> server.asset('client.mjs', 'text/javascript; charset=utf-8')
                '/client.wasm' -> server.asset('client.wasm', 'application/wasm')
                _ -> handler(request)
            }
        } else { handler(request) }
    })
}
