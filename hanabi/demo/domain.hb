use std -> *

type PostId = int[id -> id > 0]
type Title = String[s -> 1 <= len(s) && len(s) <= 120]
type Body = String[s -> 1 <= len(s) && len(s) <= 4000]

struct Draft { title: Title, body: Body }
struct Post { id: PostId, title: Title, body: Body }
enum Error { Invalid(String), Missing(PostId), Storage(String) }

effect Failure { fn fail(error: Error) -> never }

trait Posts {
    fn list(self) -> Vec<Post> ! Failure
    fn create(self, draft: Draft) -> Post ! Failure
    fn update(self, id: PostId, draft: Draft) -> Post ! Failure
}

fn draft(title: String, body: String) -> Draft ! Failure {
    let title = match refine<Title>(trim(title)) { Ok(value) -> value Err(_) -> fail(Invalid('タイトルは1〜120文字で入力してください')) }
    let body = match refine<Body>(trim(body)) { Ok(value) -> value Err(_) -> fail(Invalid('本文は1〜4000文字で入力してください')) }
    Draft { title: title, body: body }
}
fn list_posts() -> Vec<Post> ! Failure
with Posts(store) { store.list() }

fn create_post(input: Draft) -> Post ! Failure
with Posts(store) { store.create(input) }

fn update_post(id: PostId, input: Draft) -> Post ! Failure
with Posts(store) { store.update(id, input) }
