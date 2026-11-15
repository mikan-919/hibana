// examples/web の hanabi（syntax.hb の route と lib.hb）を、要求を作って呼ぶ
use web
use '../../../examples/web/syntax' -> *
use '../../../examples/web/lib' -> { hanabi } -> { Response, Clock }

fn make() -> (web.Request) -> web.Response {
    hanabi(
        route {
            GET '/' {
                Response::text('Hello!')
            }
            GET '/status' {
                Response::json({
                    status: 'OK'
                    code: 200
                })
            }
            GET '/{id}/status' {
                Response::json({
                    status: 'OK'
                    code: 200
                    message: `Hello! {id.get().unwrap()}`
                })
            }
            @durable_object('ACCOUNT') -> account
            GET `/{id}/status?time={time: 'day' | 'night'}` {
                let id = id.get().unwrap()
                let account = account.get(id)
                account.stamp(Clock::now())
                Response::json({
                    status: 'OK'
                    code: 200
                    message: `{if time.get().unwrap() == 'day' {'Hello'} else {'Good evening'}}! {id}`
                })
            }
        }
    )
}

fn show(app: (web.Request) -> web.Response, path: str, query: str) {
    let r = web.request('GET', path)
    let res = app(web.Request { method: r.method, path: r.path, query: query, headers: r.headers, body: r.body })
    put(`{res.status} {res.content_type} {res.body}`)
}

fn main() {
    let app = make()
    show(app, '/', '')
    show(app, '/status', '')
    show(app, '/u7/status', '')
    show(app, '/u7/status', 'time=day')
    show(app, '/u7/status', 'time=night')
    show(app, '/u7/status', 'time=noon')
    show(app, '/nope', '')
}
