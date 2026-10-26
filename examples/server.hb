// HTTP サーバーの例（JS ターゲットだけ: hibana js examples/server.hb > server.mjs && node server.mjs）
// ルートの本体はデータの置き場を Need<Db> の効果で受け取り、main で本物の置き場を渡す
use web

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
    }
    let db = Db { users: ["alice", "bob"] }
    let app = fn(req: web.Request) -> web.Response {
        handle api(req) {
            need() => resume(db)
        }
    }
    // 試しに1回呼ぶ（サーバーを立てずに確かめられる）
    print(app(web.Request { method: "GET", path: "/users/alice", body: "" }).body)
    web.serve(8080, app)
}
