use std -> *
use '../json.hb' -> { ToJson }
use '../storage.hb'
use './domain.hb' -> *
use './codec.hb'

struct FilePosts { path: String }
struct MemoryPosts { posts: Ref<Vec<Post>> }

fn load(path: String) -> Vec<Post> ! Failure {
    match storage.read_json(path) { Ok(value) -> codec.posts(value) Err(e) -> fail(Storage(e)) }
}
fn persist(path: String, posts: Vec<Post>) ! Failure {
    match storage.write_json(path, posts.to_json()) { Ok(_) -> {} Err(e) -> fail(Storage(e)) }
}
fn append(posts: Vec<Post>, d: Draft) -> Post {
    let mut next: PostId = 1
    for post in posts { if post.id >= next { next = post.id + 1 } }
    Post { id: next, title: d.title, body: d.body }
}
fn replaced(posts: Vec<Post>, id: PostId, draft: Draft) -> Vec<Post> ! Failure {
    let mut result: Vec<Post> = []
    let mut found = false
    for post in posts {
        if post.id == id { result = push(result, Post { id: id, title: draft.title, body: draft.body }); found = true }
        else { result = push(result, post) }
    }
    if !found { fail(Missing(id)) }
    result
}
impl Posts for FilePosts {
    fn list(self) -> Vec<Post> ! Failure { load(self.path) }
    fn create(self, draft: Draft) -> Post ! Failure {
        let saved = load(self.path)
        let post = append(saved, draft)
        persist(self.path, push(saved, post))
        post
    }
    fn update(self, id: PostId, draft: Draft) -> Post ! Failure {
        persist(self.path, replaced(load(self.path), id, draft))
        Post { id: id, title: draft.title, body: draft.body }
    }
}
impl Posts for MemoryPosts {
    fn list(self) -> Vec<Post> { read(self.posts) }
    fn create(self, draft: Draft) -> Post {
        let saved = read(self.posts)
        let post = append(saved, draft)
        write(self.posts, push(saved, post))
        post
    }
    fn update(self, id: PostId, draft: Draft) -> Post ! Failure {
        write(self.posts, replaced(read(self.posts), id, draft))
        Post { id: id, title: draft.title, body: draft.body }
    }
}
