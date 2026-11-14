// Web アプリのためのライブラリ。use web で使う
//
//   let app = routes {
//       GET '/user/{id}' { ok('user ' + id) }
//   }
//   let res = app(request('GET', '/user/42'))
//
// ルートの本体では、要求を変数 req として使える（req.body、header(req, 'x')、query(req, 'q')）
//
// routes ! Async, Need<Db> { ... } のように、ルートの本体で起こす効果を書ける。
// serve(port, app) は Node.js の HTTP サーバー、worker(app) は Cloudflare Workers の fetch として app を動かす（JS ターゲットだけ）
//
// JSX（<div class={c}>Hello {name}</div>）は HTML の文字列になる

use std
use std.{Option, Some, None}

// ---- リクエストと応答 ----

struct Header { name: str, value: str }
// query は ? の後ろ（? は含まない）。ヘッダーの名前は小文字
struct Request { method: str, path: str, query: str, headers: Vec<Header>, body: str }
// headers は追加順を保つ。同名の Set-Cookie も別々に送る
struct Response { status: int, content_type: str, body: str, headers: Vec<Header> }

fn ok(body: str) -> Response {
    Response { status: 200, content_type: 'text/plain', body: body, headers: [] }
}

fn html(body: str) -> Response {
    Response { status: 200, content_type: 'text/html', body: body, headers: [] }
}

// 本文が JSON の応答
fn json(body: str) -> Response {
    Response { status: 200, content_type: 'application/json', body: body, headers: [] }
}

fn status(code: int, body: str) -> Response {
    Response { status: code, content_type: 'text/plain', body: body, headers: [] }
}

// 応答ヘッダーを追加する。既存のヘッダーは残す
fn with_header(res: Response, name: str, value: str) -> Response {
    Response { status: res.status, content_type: res.content_type, body: res.body, headers: push(res.headers, Header { name: name, value: value }) }
}

// 保存などの後に別のページを GET で開く
fn redirect(location: str) -> Response {
    with_header(status(303, ''), 'Location', location)
}

// 試しに呼ぶための要求。ヘッダーと本文は空
fn request(method: str, path: str) -> Request {
    let hs: Vec<Header> = []
    Request { method: method, path: path, query: '', headers: hs, body: '' }
}

// ヘッダーの値。無ければ ''
fn header(req: Request, name: str) -> str {
    let mut v = ''
    for h in req.headers {
        if h.name == name { v = h.value }
    }
    v
}

// クエリの値（a=1&b=2 の a）。無ければ ''
// ponytail: % エスケープは戻さない
fn query(req: Request, name: str) -> str {
    let mut v = ''
    for p in split(req.query, '&') {
        let kv = split(p, '=')
        if len(kv) == 2 && kv[0] == name { v = kv[1] }
    }
    v
}

fn not_found() -> Response {
    Response { status: 404, content_type: 'text/plain', body: 'Not Found', headers: [] }
}

// ---- 安全なHTMLとフォーム ----

// テキスト/引用符で囲んだ属性値用。JS/CSS/URLの安全性は別に検査する
fn escape_html(s: str) -> str {
    let mut out = ''
    for c in chars(s) {
        out = out + if c == '&' { '&amp;' }
        else if c == '<' { '&lt;' }
        else if c == '>' { '&gt;' }
        else if c == '"' { '&quot;' }
        else if c == '\'' { '&#39;' }
        else { c }
    }
    out
}

fn form_hex(c: str) -> int {
    let n = code(c)
    if n >= 48 && n <= 57 { n - 48 }
    else if n >= 65 && n <= 70 { n - 55 }
    else if n >= 97 && n <= 102 { n - 87 }
    else { -1 }
}

// iからの%HH。不完全なエスケープは-1
fn form_byte(s: str, i: int) -> int {
    if std.char_at(s, i) != '%' { return -1 }
    let a = form_hex(std.char_at(s, i + 1))
    let b = form_hex(std.char_at(s, i + 2))
    if a < 0 || b < 0 { -1 } else { a * 16 + b }
}

fn form_decode(s: str) -> Option<str> {
    let mut out = ''
    let mut i = 0
    while i < len(s) {
        let c = std.char_at(s, i)
        if c == '+' {
            out = out + ' '
            i = i + 1
        } else if c == '%' {
            let first = form_byte(s, i)
            if first < 0 { return None }
            let mut n = first
            let mut count = 0
            let mut minimum = 0
            if first >= 194 && first <= 223 { n = first - 192; count = 1; minimum = 128 }
            else if first >= 224 && first <= 239 { n = first - 224; count = 2; minimum = 2048 }
            else if first >= 240 && first <= 244 { n = first - 240; count = 3; minimum = 65536 }
            else if first >= 128 { return None }
            i = i + 3
            let mut j = 0
            while j < count {
                let b = form_byte(s, i)
                if b < 128 || b > 191 { return None }
                n = n * 64 + b - 128
                i = i + 3
                j = j + 1
            }
            if n < minimum || n > 1114111 || (n >= 55296 && n <= 57343) { return None }
            out = out + char_from_codepoint(n)
        } else {
            out = out + c
            i = i + 1
        }
    }
    Some(out)
}

// 不正%/UTF8と、decode後に同名となる重複フィールドを拒否する
fn form_fields(body: str) -> Option<Vec<Header>> {
    let mut fields: Vec<Header> = []
    if body == '' { return Some(fields) }
    for pair in split(body, '&') {
        let mut boundary = len(pair)
        let mut i = 0
        while i < len(pair) {
            if std.char_at(pair, i) == '=' { boundary = i; i = len(pair) }
            else { i = i + 1 }
        }
        let raw_name = slice(pair, 0, boundary)
        let raw_value = if boundary < len(pair) { slice(pair, boundary + 1, len(pair)) } else { '' }
        match form_decode(raw_name) {
            None => return None
            Some(name) => {
                for field in fields { if field.name == name { return None } }
                match form_decode(raw_value) {
                    None => return None
                    Some(value) => { fields = push(fields, Header { name: name, value: value }) }
                }
            }
        }
    }
    Some(fields)
}

fn form_valid(body: str) -> bool {
    match form_fields(body) { Some(fields) => true, None => false }
}

// 空の値はSome('')、欠落またはフォーム全体が不正ならNone
fn form_value(body: str, name: str) -> Option<str> {
    match form_fields(body) {
        None => None
        Some(fields) => {
            for field in fields { if field.name == name { return Some(field.value) } }
            None
        }
    }
}

// ---- JSX ----

syntax JsxAttr = name:Ident '=' '{' value:Expr '}'
syntax JsxElement =
    '<' ~ tag:Ident attrs:JsxAttr* '>'
    children:JsxChild*
    '</' ~ close:Ident ~ '>'
syntax JsxChild =
    | el:JsxElement
    | '{' expr:Expr '}'
    | text:Text

// 子の要素はそのまま差し込み、展開結果の中で jsx がもう一度展開する
macro jsx (el:JsxElement) -> Expr {
    let tag = str_of(el.tag)
    if tag != str_of(el.close) {
        error('閉じタグ ' + str_of(el.close) + ' が ' + tag + ' と合わない')
    }
    let mut acc = quote { '<' + ${tag} }
    let mut i = 0
    while i < len(el.attrs) {
        let a = at(el.attrs, i)
        acc = quote { $acc + ' ' + ${str_of(a.name)} + '="' + to_str(${a.value}) + '"' }
        i = i + 1
    }
    acc = quote { $acc + '>' }
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
    quote { $acc + '</' + ${tag} + '>' }
}

// ---- routes ----

syntax Annotation = '@' ~ name:Ident '(' args:ExprList? ')' '->' alias:Ident
syntax ExprList = items:(Expr % ',')
syntax Route = annotations:Annotation* method:Ident path:String body:Block
syntax RouteEffects = '!' items:(Expr % ',')

// ルートの並びを (Request) -> Response の関数にする。どれにも合わなければ not_found()
// パスの {名前} はその名前の変数になり、注釈 @f(引数) -> x は let x = f(引数) になる
macro routes ('routes' effs:RouteEffects? '{' rs:Route* '}') -> Expr {
    let mut acc = quote { not_found() }
    let mut i = len(rs)
    while i > 0 {
        i = i - 1
        let r = at(rs, i)
        let segs = split(str_of(r.path), '/')
        let mut cond = quote { req.method == ${str_of(r.method)} && len(parts) == ${len(segs)} }
        // 本体から見える req（衛生性で隠れた引数の req を指す）
        let mut binds = quote { let ${ident('req')} = req; }
        let mut k: int[x -> x >= 0] = 0
        while k < len(segs) {
            let s = segs[k]
            if starts_with(s, '{') {
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
        quote { fn(req: Request) -> Response { let parts = split(req.path, '/'); $acc } }
    } else {
        quote { fn(req: Request) -> Response ! $( $effs.items ),* { let parts = split(req.path, '/'); $acc } }
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
