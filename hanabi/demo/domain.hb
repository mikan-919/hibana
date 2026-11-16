use std -> *
use '../json.hb' -> *
use '../endpoint.hb' -> { Fault }

scalar PostId(int[n -> n > 0])
scalar Title(String[s -> 1 <= len(s) && len(s) <= 120])
scalar Body(String[s -> 1 <= len(s) && len(s) <= 4000])
record Form { title: String, body: String }
record Draft { title: Title, body: Body }
record Post { id: PostId, title: Title, body: Body }

fn validate(form: Form) -> Result<Draft, Fault> {
    let normalized = Form { title: trim(form.title), body: trim(form.body) }
    match decode<Draft>(normalized.encode()) {
        Ok(draft) -> Ok(draft)
        Err(message) -> Err(Fault { status: 422, message: message })
    }
}
