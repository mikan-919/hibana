use std -> *
use '../client.hb' -> { Remote, query, send, mount }
use '../client.hb'
use '../dom.hb'
use '../view.hb' -> { Html, text, each }
use './domain.hb' -> *
use './codec.hb' -> { Edit }
use './codec.hb'

fn notice(message: String, error: int) { dom.notice('#notice', message, error) }
fn busy(value: int) { dom.busy('#app', value) }
fn load_posts() ! Remote { busy(1); dom.set_state('pending', 'load'); query('/api/posts') }

fn shell() -> Html {
    view div(class = 'shell') {
        view header(class = 'topbar') { view a(class = 'brand', href = '/') { text('hanabi'); view span(class = 'brand-section') { text('投稿管理') } } };
        view main {
            view div(class = 'page-intro') { view div { view p(class = 'eyebrow') { text('WORKSPACE') }; view h1 { text('投稿') }; view p(class = 'intro-copy') { text('投稿の作成と編集。保存した内容は一覧に反映されます。') } } };
            view div(id = 'notice', class = 'notice', role = 'status', aria_live = 'polite') { text('') };
            view div(class = 'workspace') {
                view section(class = 'posts-panel', aria_labelledby = 'posts-heading') {
                    view div(class = 'panel-heading') { view h2(id = 'posts-heading') { text('投稿一覧') }; view span(class = 'count', id = 'post-count') { text('0 件') } };
                    view div(id = 'posts', class = 'posts') { render_posts([]) }
                };
                view aside(class = 'editor-panel', aria_labelledby = 'editor-heading') {
                    view div(class = 'editor-heading') { view h2(id = 'editor-heading') { text('新しい投稿') } };
                    view form(id = 'post-form', data_hanabi_submit = 'submit', novalidate = 'novalidate') {
                        view input(id = 'post-id', 'type' = 'hidden', value = '0') { text('') };
                        view div(class = 'field') { view label('for' = 'title') { text('タイトル'); view span { text('必須') } }; view input(id = 'title', maxlength = '120', placeholder = '投稿のタイトル', aria_describedby = 'title-help') { text('') }; view p(id = 'title-help', class = 'field-help') { text('120文字以内') } };
                        view div(class = 'field') { view label('for' = 'body') { text('本文'); view span { text('必須') } }; view textarea(id = 'body', maxlength = '4000', placeholder = '伝えたいことを書いてください', aria_describedby = 'body-help') { text('') }; view p(id = 'body-help', class = 'field-help') { text('4000文字以内') } };
                        view div(class = 'form-actions') { view button(class = 'primary', 'type' = 'submit') { view span(id = 'save-label') { text('投稿を保存') } }; view button(class = 'secondary', 'type' = 'button', data_hanabi_call = 'cancel') { text('入力をクリア') } };
                        view p(class = 'save-note') { text('投稿はこのサーバーに保存されます。') }
                    }
                }
            }
        }
    }
}
fn render_posts(posts: Vec<Post>) -> Html {
    if len(posts) == 0 {
        view div(class = 'empty') { view h3 { text('投稿はまだありません') }; view p { text('フォームから最初の投稿を作成してください。') } }
    } else {
        each(posts, fn(post) {
            view article(class = 'post') {
                view div(class = 'post-meta') { view span { text('POST / ' + post.id.to_string()) }; view button(class = 'edit-button', 'type' = 'button', data_hanabi_call = 'edit', data_id = post.id.to_string(), aria_label = post.title + 'を編集') { text('編集') } };
                view h3 { text(post.title) };
                view p(class = 'post-body') { text(post.body) }
            }
        })
    }
}
fn ui(action: () -> unit ! Remote, Failure) {
    client.run(fn() ! Remote {
        handle action() { fail(error) -> { busy(0); match error { Invalid(message) -> notice(message, 1) Missing(id) -> notice('投稿 ' + id.to_string() + ' が見つからない', 1) Storage(message) -> notice(message, 1) } } }
    })
}
fn clear_form() {
    dom.set_value('#post-id', '0'); dom.set_value('#title', ''); dom.set_value('#body', '')
    dom.text('#editor-heading', '新しい投稿'); dom.text('#save-label', '投稿を保存')
}
fn start() { ui(fn() ! Remote, Failure { mount('#app', shell()); load_posts() }) }
fn submit() {
    ui(fn() ! Remote, Failure {
        let input = draft(dom.value('#title'), dom.value('#body'))
        let id = match parse_int(dom.value('#post-id')) { Ok(value) -> value Err(_) -> 0 }
        busy(1)
        if id == 0 { dom.set_state('pending', 'create'); send('POST', '/api/posts', input) }
        else {
            let id = match refine<PostId>(id) { Ok(value) -> value Err(e) -> fail(Invalid(e)) }
            dom.set_state('pending', 'edit'); send('PUT', '/api/posts', Edit { id: id, draft: input })
        }
    })
}
fn edit(id: int) {
    ui(fn() ! Remote, Failure {
        for post in codec.parse_posts(dom.state('posts')) {
            if post.id == id {
                dom.set_value('#post-id', id.to_string()); dom.set_value('#title', post.title); dom.set_value('#body', post.body)
                dom.text('#editor-heading', '投稿を編集'); dom.text('#save-label', '変更を保存')
                notice('投稿 ' + id.to_string() + ' を編集中', 0); dom.focus('#title')
            }
        }
    })
}
fn cancel() { clear_form(); notice('入力をクリアしました', 0); dom.focus('#title') }
fn receive(status: int, body: String) {
    busy(0)
    ui(fn() ! Remote, Failure {
        if status >= 400 { fail(Invalid(codec.error_message(body))) }
        let pending = dom.state('pending')
        if pending == 'load' {
            let posts = codec.parse_posts(body)
            dom.set_state('posts', body); mount('#posts', render_posts(posts)); dom.text('#post-count', len(posts).to_string() + ' 件')
        } else {
            clear_form(); notice(if pending == 'create' { '投稿を保存しました' } else { '変更を保存しました' }, 0)
            load_posts()
        }
    })
}
fn network_error(message: String) { busy(0); notice(message, 1) }
