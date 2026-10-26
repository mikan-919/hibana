// web ライブラリ（lib/web.hb）を使う例
import web

fn durable_objects(name: str) -> str { "stub(" + name + ")" }

fn main() {
    let user = "Alice"
    let page = <div class={"card"}>
        Hello {user}!
        <Icon name={"star"}></Icon>
    </div>
    print(page)

    let app = routes {
        GET "/" {
            html(<h1>Welcome</h1>)
        }

        GET "/user/{id}" {
            ok("user " + id)
        }

        @durable_objects("ACCOUNT") -> account
        POST "/post/{user}/{post}" {
            ok(account + ": post " + post + " by " + user)
        }
    }
    for path in ["/", "/user/42", "/nothing"] {
        let res = app(Request { method: "GET", path: path })
        print(to_str(res.status) + " " + res.body)
    }
    print(app(Request { method: "POST", path: "/post/bob/7" }))
}
