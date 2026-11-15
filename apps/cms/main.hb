// 記事のルート・入力検証・HTML は Hibana。ファイルと暗号処理だけ Node のホストへ渡す。
use web
use std -> *

extern {
    fn cms_load() -> str
    fn cms_save(data: str)
    fn cms_login(password: str) -> str
    fn cms_authorized(cookie: str) -> bool
    fn cms_csrf(cookie: str) -> str
    fn cms_logout(cookie: str)
    fn cms_port() -> int
    fn cms_cookie_secure() -> bool
}

struct Post { id: str, title: str, body: str, published: bool }

fn load_posts() -> Vec<Post> {
    let parsed = match parse_json(cms_load()) {
        Some(j) => j
        None => { panic('保存された記事の JSON を読めない'); JNull }
    }
    let mut posts: Vec<Post> = []
    match parsed {
        JArr(items) => {
            for item in items {
                let published = match field(item, 'published') {
                    Some(JBool(b)) => b
                    _ => false
                }
                posts = push(posts, Post {
                    id: str_or(field(item, 'id'), ''),
                    title: str_or(field(item, 'title'), ''),
                    body: str_or(field(item, 'body'), ''),
                    published: published
                })
            }
        }
        _ => { panic('記事の保存形式が違う') }
    }
    posts
}

fn save_posts(db: Ref<Vec<Post>>, posts: Vec<Post>) {
    let mut items: Vec<Json> = []
    for p in posts {
        items = push(items, JObj([
            Field { key: 'id', value: JStr(p.id) },
            Field { key: 'title', value: JStr(p.title) },
            Field { key: 'body', value: JStr(p.body) },
            Field { key: 'published', value: JBool(p.published) }
        ]))
    }
    // ディスクへの保存が成功するまでは、メモリー内の記事を変えない。
    cms_save(stringify(JArr(items)))
    write(db, posts)
}

fn post_by_id(posts: Vec<Post>, id: str) -> Option<Post> {
    let mut result: Option<Post> = None
    for p in posts {
        if p.id == id { result = Some(p) }
    }
    result
}

// 記事数 + 1 までに必ず空きがある。削除した番号は再利用する。
fn unused_id(posts: Vec<Post>) -> str {
    for n in 1..len(posts) + 2 {
        let id = to_str(n)
        match post_by_id(posts, id) {
            None => { return id }
            Some(p) => {}
        }
    }
    panic('記事番号を割り当てられない')
    ''
}

fn form(req: web.Request, key: str) -> str {
    unwrap_or(web.form_value(req.body, key), '')
}

fn logged_in(req: web.Request) -> bool {
    cms_authorized(web.header(req, 'cookie'))
}

fn csrf_field(req: web.Request) -> str {
    '<input type="hidden" name="_csrf" value="' + web.escape_html(cms_csrf(web.header(req, 'cookie'))) + '">'
}

fn valid_csrf(req: web.Request) -> bool {
    let expected = cms_csrf(web.header(req, 'cookie'))
    expected != '' && form(req, '_csrf') == expected
}

fn valid_form(req: web.Request) -> bool {
    let parts = split(web.header(req, 'content-type'), ';')
    let content_type = if len(parts) > 0 { trim(parts[0]) } else { '' }
    content_type == 'application/x-www-form-urlencoded' && web.form_valid(req.body)
}

fn page(title: str, content: str) -> web.Response {
    web.html('<!doctype html><html lang="ja"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>' + web.escape_html(title) + ' · Hibana CMS</title><style>body{font:17px system-ui,sans-serif;max-width:850px;margin:40px auto;padding:0 20px;line-height:1.7;color:#222}nav{display:flex;gap:20px;border-bottom:1px solid #ddd;padding-bottom:15px}input[type=text],input[type=password],textarea{box-sizing:border-box;width:100%;font:inherit;padding:10px;border:1px solid #aaa;border-radius:4px}textarea{min-height:320px}button{font:inherit;padding:8px 20px;cursor:pointer}label{display:block;margin:18px 0}article{white-space:pre-wrap;overflow-wrap:anywhere}li{margin:12px 0}.muted{color:#666}.danger{color:#a00}</style></head><body><nav><a href="/">Hibana CMS</a><a href="/admin">記事管理</a></nav><main><h1>' + web.escape_html(title) + '</h1>' + content + '</main></body></html>')
}

fn login_page() -> web.Response {
    page('ログイン', '<form method="post" action="/login"><label>管理パスワード<input type="password" name="password" autocomplete="current-password" required maxlength="1024"></label><button type="submit">ログイン</button></form>')
}

fn login(req: web.Request) -> web.Response {
    if !valid_form(req) { return web.status(400, 'フォームが不正です') }
    let password = form(req, 'password')
    if len(password) > 1024 { return web.status(400, 'パスワードが長すぎます') }
    let token = cms_login(password)
    if token == '' { return web.status(401, 'ログインできません。パスワードを確認するか、しばらく待ってください') }
    let secure = if cms_cookie_secure() { '; Secure' } else { '' }
    web.with_header(web.redirect('/admin'), 'Set-Cookie', 'cms_session=' + token + '; HttpOnly; SameSite=Strict; Path=/; Max-Age=28800' + secure)
}

fn index(posts: Vec<Post>) -> web.Response {
    let mut content = '<ul>'
    let mut count = 0
    for p in posts {
        if p.published {
            content = content + '<li><a href="/posts/' + web.escape_html(p.id) + '">' + web.escape_html(p.title) + '</a></li>'
            count = count + 1
        }
    }
    content = content + '</ul>'
    if count == 0 { content = '<p class="muted">公開された記事はまだありません。</p>' }
    page('記事', content)
}

fn show_post(posts: Vec<Post>, id: str) -> web.Response {
    match post_by_id(posts, id) {
        Some(p) => {
            if p.published { page(p.title, '<article>' + web.escape_html(p.body) + '</article>') } else { web.not_found() }
        }
        None => web.not_found()
    }
}

fn admin(req: web.Request, posts: Vec<Post>) -> web.Response {
    if !logged_in(req) { return web.redirect('/login') }
    let mut content = '<p><a href="/admin/new">新しい記事を書く</a></p><ul>'
    for p in posts {
        let label = if p.published { '公開' } else { '下書き' }
        content = content + '<li><a href="/admin/posts/' + web.escape_html(p.id) + '">' + web.escape_html(p.title) + '</a> <span class="muted">' + label + '</span></li>'
    }
    content = content + '</ul><form method="post" action="/logout">' + csrf_field(req) + '<button type="submit">ログアウト</button></form>'
    page('記事管理', content)
}

fn editor(req: web.Request, p: Post, creating: bool) -> web.Response {
    let action = if creating { '/admin/posts' } else { '/admin/posts/' + p.id }
    let checked = if p.published { ' checked' } else { '' }
    let mut content = '<form method="post" action="' + web.escape_html(action) + '">' + csrf_field(req) + '<label>タイトル<input type="text" name="title" required maxlength="200" value="' + web.escape_html(p.title) + '"></label><label>本文<textarea name="body" maxlength="100000">' + web.escape_html(p.body) + '</textarea></label><label><input type="checkbox" name="published" value="1"' + checked + '>公開する</label><button type="submit">保存</button> <a href="/admin">戻る</a></form>'
    if !creating {
        content = content + '<hr><form method="post" action="' + web.escape_html(action) + '/delete">' + csrf_field(req) + '<button type="submit" class="danger">この記事を削除</button></form>'
    }
    page(if creating { '新しい記事' } else { '記事を編集' }, content)
}

fn edit_post(req: web.Request, posts: Vec<Post>, id: str) -> web.Response {
    if !logged_in(req) { return web.redirect('/login') }
    match post_by_id(posts, id) {
        Some(p) => editor(req, p, false)
        None => web.not_found()
    }
}

fn mutate(req: web.Request, db: Ref<Vec<Post>>, id: str, deleting: bool) -> web.Response {
    if !logged_in(req) { return web.status(401, 'ログインが必要です') }
    if !valid_form(req) { return web.status(400, 'フォームが不正です') }
    if !valid_csrf(req) { return web.status(403, 'CSRF トークンが不正です') }
    let posts = read(db)
    if id != '' {
        match post_by_id(posts, id) {
            None => { return web.not_found() }
            Some(p) => {}
        }
    }
    let title = trim(form(req, 'title'))
    let body = form(req, 'body')
    let published = form(req, 'published') == '1' || form(req, 'published') == 'on'
    if !deleting && (title == '' || len(title) > 200 || len(body) > 100000) {
        return web.status(400, 'タイトルは1〜200文字、本文は100000文字までです')
    }
    if id == '' && len(posts) >= 1000 { return web.status(400, '記事は1000件までです') }
    let mut updated: Vec<Post> = []
    for p in posts {
        if p.id == id {
            if !deleting { updated = push(updated, Post { id: id, title: title, body: body, published: published }) }
        } else {
            updated = push(updated, p)
        }
    }
    if id == '' {
        updated = push(updated, Post { id: unused_id(posts), title: title, body: body, published: published })
    }
    save_posts(db, updated)
    web.redirect('/admin')
}

fn logout(req: web.Request) -> web.Response {
    if !logged_in(req) { return web.status(401, 'ログインが必要です') }
    if !valid_form(req) { return web.status(400, 'フォームが不正です') }
    if !valid_csrf(req) { return web.status(403, 'CSRF トークンが不正です') }
    cms_logout(web.header(req, 'cookie'))
    let secure = if cms_cookie_secure() { '; Secure' } else { '' }
    web.with_header(web.redirect('/login'), 'Set-Cookie', 'cms_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0' + secure)
}

fn secure_response(res: web.Response) -> web.Response {
    let r = web.with_header(res, 'X-Content-Type-Options', 'nosniff')
    let r = web.with_header(r, 'Referrer-Policy', 'same-origin')
    let r = web.with_header(r, 'Cache-Control', 'no-store')
    web.with_header(r, 'Content-Security-Policy', 'default-src \'none\'; style-src \'unsafe-inline\'; form-action \'self\'; base-uri \'none\'; frame-ancestors \'none\'')
}

fn main() ! Async {
    let db = new_ref(load_posts())
    let api = routes {
        GET '/' { index(read(db)) }
        GET '/posts/{id}' { show_post(read(db), id) }
        GET '/login' { login_page() }
        POST '/login' { login(req) }
        POST '/logout' { logout(req) }
        GET '/admin' { admin(req, read(db)) }
        GET '/admin/new' {
            if logged_in(req) { editor(req, Post { id: '', title: '', body: '', published: false }, true) } else { web.redirect('/login') }
        }
        GET '/admin/posts/{id}' { edit_post(req, read(db), id) }
        POST '/admin/posts' { mutate(req, db, '', false) }
        POST '/admin/posts/{id}' { mutate(req, db, id, false) }
        POST '/admin/posts/{id}/delete' { mutate(req, db, id, true) }
    }
    let app = fn(req: web.Request) -> web.Response { secure_response(api(req)) }
    web.serve(cms_port(), app)
}
