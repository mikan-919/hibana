// Web アプリのためのライブラリ。use web で使う
//
//   let app = routes {
//       GET "/user/{id}" { ok("user " + id) }
//   }
//   let res = app(Request { method: "GET", path: "/user/42", body: "" })
//
// routes ! Async, Need<Db> { ... } のように、ルートの本体で起こす効果を書ける。
// serve(port, app) は Node.js の HTTP サーバー、worker(app) は Cloudflare Workers の fetch として app を動かす（JS ターゲットだけ）
//
// JSX（<div class={c}>Hello {name}</div>）は HTML の文字列になる

// ---- リクエストと応答 ----

type Request = { method: str, path: str, body: str }
type Response = { status: int, content_type: str, body: str }

fn ok(body: str) -> Response {
    Response { status: 200, content_type: "text/plain", body: body }
}

fn html(body: str) -> Response {
    Response { status: 200, content_type: "text/html", body: body }
}

fn not_found() -> Response {
    Response { status: 404, content_type: "text/plain", body: "Not Found" }
}

// ---- JSX ----

syntax JsxAttr = name:Ident "=" "{" value:Expr "}"
syntax JsxElement =
    "<" ~ tag:Ident attrs:JsxAttr* ">"
    children:JsxChild*
    "</" ~ close:Ident ~ ">"
syntax JsxChild =
    | el:JsxElement
    | "{" expr:Expr "}"
    | text:Text

// 子の要素はそのまま差し込み、展開結果の中で jsx がもう一度展開する
macro jsx (el:JsxElement) -> Expr {
    let tag = str_of(el.tag)
    if tag != str_of(el.close) {
        error("閉じタグ " + str_of(el.close) + " が " + tag + " と合わない")
    }
    let mut acc = quote { "<" + ${tag} }
    let mut i = 0
    while i < len(el.attrs) {
        let a = at(el.attrs, i)
        acc = quote { $acc + " " + ${str_of(a.name)} + "=\"" + to_str(${a.value}) + "\"" }
        i = i + 1
    }
    acc = quote { $acc + ">" }
    let mut j = 0
    while j < len(el.children) {
        let c = at(el.children, j)
        if !is_null(c.text) {
            acc = quote { $acc + ${str_of(c.text)} }
        } else if !is_null(c.expr) {
            acc = quote { $acc + to_str(${c.expr}) }
        } else {
            acc = quote { $acc + ${c.el} }
        }
        j = j + 1
    }
    quote { $acc + "</" + ${tag} + ">" }
}

// ---- routes ----

syntax Annotation = "@" ~ name:Ident "(" args:ExprList? ")" "->" alias:Ident
syntax ExprList = items:(Expr % ",")
syntax Route = annotations:Annotation* method:Ident path:String body:Block
syntax RouteEffects = "!" items:(Expr % ",")

// ルートの並びを (Request) -> Response の関数にする。どれにも合わなければ not_found()
// パスの {名前} はその名前の変数になり、注釈 @f(引数) -> x は let x = f(引数) になる
macro routes ("routes" effs:RouteEffects? "{" rs:Route* "}") -> Expr {
    let mut acc = quote { not_found() }
    let mut i = len(rs)
    while i > 0 {
        i = i - 1
        let r = at(rs, i)
        let segs = split(str_of(r.path), "/")
        let mut cond = quote { req.method == ${str_of(r.method)} && len(parts) == ${len(segs)} }
        let mut binds = quote { }
        let mut k: int[x -> x >= 0] = 0
        while k < len(segs) {
            let s = segs[k]
            if starts_with(s, "{") {
                binds = quote { $binds let ${ident(slice(s, 1, len(s) - 1))} = parts[${k}]; }
            } else {
                cond = quote { $cond && parts[${k}] == ${s} }
            }
            k = k + 1
        }
        let mut j = 0
        while j < len(r.annotations) {
            let a = at(r.annotations, j)
            binds = quote { $binds let ${a.alias} = ${a.name}(${a.args}); }
            j = j + 1
        }
        acc = quote { if $cond { $binds ${r.body} } else { $acc } }
    }
    if is_null(effs) {
        quote { fn(req: Request) -> Response { let parts = split(req.path, "/"); $acc } }
    } else {
        quote { fn(req: Request) -> Response ! $( $effs.items ),* { let parts = split(req.path, "/"); $acc } }
    }
}

// ---- サーバー ----

// 中身は JS の出力の先頭にある（globalThis.hibana_serve と hibana_worker）
extern {
    fn hibana_serve(port: int, app: (Request) -> Response ! Async) ! Async
    fn hibana_worker(app: (Request) -> Response ! Async)
}

// Node.js の HTTP サーバーとして app を動かす。port が 0 なら空いている番号を使い、URL を表示する
fn serve(port: int, app: (Request) -> Response ! Async) ! Async {
    hibana_serve(port, app)
}

// Cloudflare Workers の fetch として app を動かす（出力の export default { fetch } から呼ぶ）
fn worker(app: (Request) -> Response ! Async) {
    hibana_worker(app)
}
