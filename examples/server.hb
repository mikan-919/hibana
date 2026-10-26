// HTTP サーバーの例（JS ターゲットだけ: hibana js examples/server.hb > server.mjs && node server.mjs）
// ルートの本体はデータの置き場を Need<Db> の効果で受け取り、main で本物の置き場を渡す
use web
use std.*

effect Need<T> {
    fn need() -> T
}

type Db = { users: Vec<str> }

fn exists(name: str) -> bool ! Need<Db> {
    let db: Db = need()
    let mut found = false
    for u in db.users {
        if u == name { found = true }
    }
    found
}

fn main() ! Async {
    let api = routes ! Need<Db> {
        GET "/" {
            web.html(<h1>hibana</h1>)
        }

        GET "/users/{name}" {
            if exists(name) { web.ok("found " + name) } else { web.not_found() }
        }

        // 一覧を JSON で返す
        GET "/users" {
            let db: Db = need()
            let mut items: Vec<Json> = []
            for u in db.users { items = push(items, JStr(u)) }
            web.json(stringify(JObj([Field { key: "users", value: JArr(items) }])))
        }

        // ルートの本体では要求を req として使える
        POST "/echo" {
            let who = web.query(req, "from")
            let agent = web.header(req, "x-agent")
            web.json(stringify(JObj([
                Field { key: "from", value: JStr(who) },
                Field { key: "agent", value: JStr(agent) },
                Field { key: "body", value: JStr(req.body) }
            ])))
        }
    }
    let db = Db { users: ["alice", "bob"] }
    let app = fn(req: web.Request) -> web.Response {
        handle api(req) {
            need() => resume(db)
        }
    }
    // 試しに1回呼ぶ（サーバーを立てずに確かめられる）
    print(app(web.request("GET", "/users/alice")).body)
    web.serve(8080, app)
}
