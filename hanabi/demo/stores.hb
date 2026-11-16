use std -> *
use '../types.hb' -> { FilePath }
use '../storage.hb' -> { Storage }
use '../json.hb' -> { Codec, decode }
use '../endpoint.hb' -> { Fault }
use './domain.hb' -> *

struct Repository { path: FilePath }
fn storage_error(message: String) -> Fault { Fault { status: 500, message: message } }
fn list(repo: Repository) -> Result<Vec<Post>, Fault>
with Storage(storage) {
    let value = match storage.load(repo.path) { Ok(value) -> value Err(e) -> return Err(storage_error(e)) }
    match value {
        None -> Ok([])
        Some(value) -> match decode<Vec<Post>>(value) { Ok(items) -> Ok(items) Err(e) -> Err(storage_error(e)) }
    }
}
fn save(repo: Repository, items: Vec<Post>) -> Result<unit, Fault>
with Storage(storage) {
    match storage.save(repo.path, items.encode()) { Ok(_) -> Ok({}) Err(e) -> Err(storage_error(e)) }
}
fn create(repo: Repository, form: Form) -> Result<Post, Fault>
with Storage(storage) {
    let draft = validate(form)?
    let items = list(repo)?
    let mut last: int[n -> n >= 0] = 0
    for post in items { if post.id.0 > last { last = post.id.0 } }
    let post = Post { id: PostId(last + 1), title: draft.title, body: draft.body }
    save(repo, push(items, post))?
    Ok(post)
}
fn update(repo: Repository, id: PostId, form: Form) -> Result<Post, Fault>
with Storage(storage) {
    let draft = validate(form)?
    let items = list(repo)?
    let post = Post { id: id, title: draft.title, body: draft.body }
    let mut updated: Vec<Post> = []
    let mut found = false
    for item in items {
        if item.id.0 == id.0 { updated = push(updated, post); found = true }
        else { updated = push(updated, item) }
    }
    if !found { return Err(Fault { status: 404, message: '投稿が見つからない' }) }
    save(repo, updated)?
    Ok(post)
}
