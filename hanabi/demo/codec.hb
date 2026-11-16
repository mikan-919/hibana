use std -> *
use '../json.hb' -> { ToJson }
use '../input.hb'
use './domain.hb' -> *
use './domain.hb'

struct Edit { id: PostId, draft: Draft }

fn posts(value: Json) -> Vec<Post> ! Failure {
    match value {
        JNull -> []
        JArr(items) -> {
            let mut result: Vec<Post> = []
            for value in items {
                let id = match refine<PostId>(int_or(field(value, 'id'), 0)) { Ok(id) -> id Err(e) -> fail(Storage(e)) }
                let d = domain.draft(str_or(field(value, 'title'), ''), str_or(field(value, 'body'), ''))
                result = push(result, Post { id: id, title: d.title, body: d.body })
            }
            result
        }
        _ -> fail(Storage('一覧を読めない'))
    }
}
fn parse_posts(text: String) -> Vec<Post> ! Failure {
    match parse_json(text) { Some(value) -> posts(value) None -> fail(Storage('一覧の応答を読めない')) }
}
fn error_message(text: String) -> String {
    match parse_json(text) { Some(value) -> str_or(field(value, 'error'), '要求を処理できない') None -> '応答を読めない' }
}

impl ToJson for Post {
    fn to_json(self) -> Json { object { id: self.id, title: self.title, body: self.body } }
}
impl ToJson for Draft {
    fn to_json(self) -> Json { object { title: self.title, body: self.body } }
}
impl ToJson for Edit {
    fn to_json(self) -> Json { object { id: self.id, title: self.draft.title, body: self.draft.body } }
}
impl input.Decode for Draft {
    fn decode(value: Json) -> Draft ! input.Rejected {
        handle domain.draft(input.text(value, 'title'), input.text(value, 'body')) {
            fail(error) -> match error {
                Invalid(message) -> input.reject('form', message)
                _ -> input.reject('form', '入力を読めない')
            }
        }
    }
}
impl input.Decode for Edit {
    fn decode(value: Json) -> Edit ! input.Rejected {
        let id = match refine<PostId>(input.integer(value, 'id')) { Ok(id) -> id Err(e) -> input.reject('id', e) }
        let draft: Draft = input.decode(value)
        Edit { id: id, draft: draft }
    }
}
