// web ライブラリ（lib/web.hb）を使う例。use web の名前は web.ok のように修飾して使う
// （use web.* なら修飾なし、use web.{ok, Request} なら挙げた名前だけ修飾なし）
use web

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
            web.html(<h1>Welcome</h1>)
        }

        GET "/user/{id}" {
            web.ok("user " + id)
        }

        @durable_objects("ACCOUNT") -> account
        POST "/post/{user}/{post}" {
            web.ok(account + ": post " + post + " by " + user)
        }
    }
    for path in ["/", "/user/42", "/nothing"] {
        let res = app(web.request("GET", path))
        print(to_str(res.status) + " " + res.body)
    }
    print(app(web.request("POST", "/post/bob/7")))
}
