use std -> *
use '../client.hb' -> { Program, Session, Change }
use '../dom.hb'
use '../view.hb' -> { Html, each }
use '../json.hb' -> { Codec, decode }
use '../endpoint.hb' -> { Fault }
use '../types.hb' -> { Status, RequestId }
use './domain.hb' -> *
use './api.hb' -> *

union Event { Reload(unit), Submit(unit), Edit(PostId), Cancel(unit) }
enum Message { Ui(Event), Loaded(Result<Vec<Post>, Fault>), Saved(Result<Post, Fault>) }
struct Model {
    items: Vec<Post>, editing: Option<PostId>, form: Form,
    busy: bool, notice: String, error: bool
}
type App = Ref<Session<Model, Message>>
fn initial() -> Model {
    Model { items: [], editing: None, form: Form { title: '', body: '' }, busy: false, notice: '', error: false }
}
fn load(model: Model) -> Change<Model, Message> {
    Change {
        state: Model { items: model.items, editing: model.editing, form: model.form, busy: true, notice: model.notice, error: model.error },
        command: client.call(list_api(), {}, {}, fn(result) { Loaded(result) })
    }
}
fn update(model: Model, message: Message) -> Change<Model, Message> ! dom.Dom {
    match message {
        Ui(event) -> match event {
            Reload(_) -> load(model)
            Submit(_) -> {
                let form = Form { title: dom.value('#title'), body: dom.value('#body') }
                let command = match model.editing {
                    None -> client.call(create_api(), {}, form, fn(result) { Saved(result) })
                    Some(id) -> client.call(update_api(), id, form, fn(result) { Saved(result) })
                }
                Change {
                    state: Model { items: model.items, editing: model.editing, form: form, busy: true, notice: '', error: false },
                    command: command
                }
            }
            Edit(id) -> {
                let mut form = model.form
                for post in model.items { if post.id.0 == id.0 { form = Form { title: post.title.0, body: post.body.0 } } }
                client.stay(Model { items: model.items, editing: Some(id), form: form, busy: false, notice: '投稿を編集中', error: false })
            }
            Cancel(_) -> client.stay(Model { items: model.items, editing: None, form: Form { title: '', body: '' }, busy: false, notice: '入力をクリアしました', error: false })
        }
        Loaded(result) -> match result {
            Ok(items) -> client.stay(Model { items: items, editing: model.editing, form: model.form, busy: false, notice: model.notice, error: model.error })
            Err(e) -> client.stay(Model { items: model.items, editing: model.editing, form: model.form, busy: false, notice: e.message, error: true })
        }
        Saved(result) -> match result {
            Ok(_) -> load(Model { items: model.items, editing: None, form: Form { title: '', body: '' }, busy: false, notice: '投稿を保存しました', error: false })
            Err(e) -> client.stay(Model { items: model.items, editing: model.editing, form: model.form, busy: false, notice: e.message, error: true })
        }
    }
}
fn view(model: Model) -> Html {
    let heading = match model.editing { None -> '新しい投稿' Some(_) -> '投稿を編集' }
    let save_label = match model.editing { None -> '投稿を保存' Some(_) -> '変更を保存' }
    let posts = if len(model.items) == 0 {
        <div class="empty"><h3>投稿はまだありません</h3><p>フォームから最初の投稿を作成してください。</p></div>
    } else {
        each(model.items, fn(post) {
            <article class="post">
                <div class="post-meta"><span>{'POST / ' + post.id.0.to_string()}</span><button class="edit-button" type="button" disabled={model.busy} onClick={Edit(post.id)} aria-label={post.title.0 + 'を編集'}>編集</button></div>
                <h3>{post.title.0}</h3><p class="post-body">{post.body.0}</p>
            </article>
        })
    }
    <div class = "shell">
    <header class = "topbar"><a class = "brand" href = "/">hanabi<span class = "brand-section">投稿管理</span></a></header>
    <main>
    <div class = "page-intro"><div><p class = "eyebrow">WORKSPACE</p><h1>投稿</h1><p class = "intro-copy">投稿の作成と編集。保存した内容は一覧に反映されます。</p></div></div>
    <div id = "notice" class = { if model.error { 'notice error' } else { 'notice' } } role = "status" aria-live = "polite"> { model.notice }</div>
    <div class = "workspace">
    <section class = "posts-panel" aria-labelledby = "posts-heading">
    <div class = "panel-heading"><h2 id = "posts-heading">投稿一覧</h2><span class = "count" id = "post-count"> { len(model.items).to_string() + ' 件' }</span></div>
    <div id = "posts" class = "posts"> { posts }</div>
    </section>
    <aside class = "editor-panel" aria-labelledby = "editor-heading">
    <div class = "editor-heading"><h2 id = "editor-heading"> { heading }</h2></div>
    <form id = "post-form" onSubmit = { Submit({}) } novalidate = "novalidate">
    <div class = "field"><label for = "title">タイトル<span>必須</span></label><input id = "title" value = { model.form.title } placeholder = "投稿のタイトル" aria-describedby = "title-help" /><p id = "title-help" class = "field-help">120文字以内</p></div>
    <div class = "field"><label for = "body">本文<span>必須</span></label><textarea id = "body" placeholder = "伝えたいことを書いてください" aria-describedby = "body-help"> { model.form.body }</textarea><p id = "body-help" class = "field-help">4000文字以内</p></div>
    <div class = "form-actions"><button class = "primary" type = "submit" disabled = { model.busy }> { save_label }</button><button class = "secondary" type = "button" disabled = { model.busy } onClick = { Cancel({}) }>入力をクリア</button></div>
    <p class = "save-note">投稿はこのサーバーに保存されます。</p>
    </form>
    </aside>
    </div>
    </main>
    </div>
}
fn bootstrap(root: String) -> App {
    let program = Program {
        initial: initial(), update: update, view: view, root: root,
        event: fn(value) { Ok(Ui(decode<Event>(value)?)) }
    }
    let app = dom.browser(fn() -> App ! dom.Dom { client.create(program) })
    client.browser(fn() ! dom.Dom, client.Schedule { client.dispatch(app, Ui(Reload({}))) })
    app
}
fn dispatch(app: App, payload: String) { client.browser(fn() ! dom.Dom, client.Schedule { client.event(app, payload) }) }
fn resolve(app: App, id: int, status: int, body: String) {
    let id = match refine<RequestId>(id) { Ok(id) -> id Err(e) -> panic(e) }
    let status = match refine<Status>(status) { Ok(status) -> status Err(e) -> panic(e) }
    client.browser(fn() ! dom.Dom, client.Schedule { client.resolve(app, id, status, body) })
}
