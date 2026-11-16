use std -> *
use '../json.hb' -> { Codec, parse, print }
use '../view.hb' -> { render }
use '../storage.hb' -> { Storage, MemoryStorage }
use '../client.hb' -> { Transport, exchange, Schedule, enqueue }
use '../endpoint.hb' -> { Reply, Fault }
use '../dom.hb' -> { Dom, value, paint, focus }
use '../types.hb' -> { Put }
use '../demo/domain.hb' -> { PostId, Title, Form, Post }
use '../demo/api.hb' -> { update_api, create_api }
use '../demo/stores.hb' -> { Repository }

fn repository_contract() {
    with Storage(MemoryStorage { files: new_ref([]) }) {
        let repo = Repository { path: 'test.json' }
        let post = match stores.create(repo, Form { title: '題', body: '本文' }) { Ok(post) -> post Err(e) -> panic(e.message) }
        assert(post.id.0 == 1)
        assert(post.title.0 == '題')
        assert(match stores.create(repo, Form { title: '', body: '文' }) { Ok(_) -> false Err(e) -> e.status == 422 })
        let saved = match stores.update(repo, post.id, Form { title: '編集', body: '更新' }) { Ok(post) -> post Err(e) -> panic(e.message) }
        let items = match stores.list(repo) { Ok(items) -> items Err(e) -> panic(e.message) }
        assert(len(items) == 1)
        for item in items { assert(item.title.0 == saved.title.0) }
    }
}
fn transport_contract() {
    let command = client.call(update_api(), PostId(7), Form { title: '題', body: '本文' }, fn(result) { result })
    let result = handle client.perform(command) {
        exchange(request) -> {
            assert(request.method == Put)
            assert(request.url == '/api/posts/7')
            resume(Reply { status: 200, body: '{"id":7,"title":"題","body":"本文"}' })
        }
    }
    assert(match result { Some(result) -> match result { Ok(post) -> post.id.0 == 7 Err(_) -> false } None -> false })
}
enum Message { Send, Accepted(Result<Post, Fault>) }
fn step(state: int, message: Message) -> client.Change<int, Message> ! Dom {
    match message {
        Send -> {
            let form = Form { title: value('title'), body: value('body') }
            client.Change { state: state, command: client.call(create_api(), {}, form, fn(result) { Accepted(result) }) }
        }
        Accepted(result) -> match result { Ok(post) -> client.stay(post.id.0) Err(_) -> client.stay(-1) }
    }
}
fn session_contract() {
    let paints = new_ref(0)
    let scheduled = new_ref(0)
    handle handle {
        let session = client.create(client.Program { initial: 0, root: 'root', update: step, view: fn(n) { <p>{n.to_string()}</p> }, event: fn(_) { Ok(Send) } })
        client.dispatch(session, Send)
        assert(len(read(session).pending) == 1)
        client.resolve(session, 1, 201, '{"id":9,"title":"題","body":"本文"}')
        assert(read(session).state == 9)
        assert(len(read(session).pending) == 0)
        client.resolve(session, 1, 201, '{"id":10,"title":"題","body":"本文"}')
        assert(read(session).state == 9)
    } {
        enqueue(id, request) -> { assert(id == 1); assert(request.url == '/api/posts'); write(scheduled, read(scheduled) + 1); resume() }
    } {
        value(selector) -> resume(if selector == 'title' { '題' } else { '本文' })
        paint(_, _) -> { write(paints, read(paints) + 1); resume() }
        focus(_) -> resume()
    }
    assert(read(scheduled) == 1)
    assert(read(paints) == 3)
}
fn view_contract() {
    assert(render(<label for="entry"><input type="text" disabled={false} />{'<script>'}</label>) == '<label for="entry"><input type="text">&lt;script&gt;</label>')
    assert(match parse<Title>('""') { Ok(_) -> false Err(_) -> true })
    assert(print(PostId(4)) == '4')
}

test '保存先の差し替えと細分化型' { repository_contract() }
test '通信効果の応答で再開する' { transport_contract() }
test '型付きセッションと通信完了' { session_contract() }
test 'JSX と生成した変換' { view_contract() }
fn main() { repository_contract(); transport_contract(); session_contract(); view_contract(); put('hanabi contracts passed') }
