use '../app.hb' -> *
use '../routes.hb'
use '../input.hb' -> { json_body, Rejected, reject }
use './domain.hb' -> *
use './codec.hb' -> { Edit }
use './stores.hb' -> { FilePosts }

fn endpoints(request: Request) -> Response ! Rejected, Failure
with Posts(store) {
    routes request {
        GET '/api/posts' -> { ok(list_posts()) }
        POST '/api/posts' -> { created(create_post(json_body<Draft>(request))) }
        PUT '/api/posts' -> {
            let input = json_body<Edit>(request)
            ok(update_post(input.id, input.draft))
        }
    }
}

fn app(request: Request) -> Response {
    with Posts(FilePosts { path: 'data/posts.json' }) {
        handle handle endpoints(request) {
            reject(field, message) -> invalid(field, message)
        } {
            fail(error) -> match error {
                Invalid(message) -> invalid('form', message)
                Missing(id) -> not_found('投稿 ' + id.to_string() + ' が見つからない')
                Storage(message) -> internal_error(message)
            }
        }
    }
}

fn main() { run(8787, app) }
