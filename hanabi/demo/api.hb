use '../endpoint.hb' -> *
use './domain.hb' -> { PostId, Form, Post }

api {
    list_api GET '/api/posts' (unit, unit) -> Vec<Post> ! Fault = 200
    create_api POST '/api/posts' (unit, Form) -> Post ! Fault = 201
    update_api PUT '/api/posts/{id}' (PostId, Form) -> Post ! Fault = 200
}
