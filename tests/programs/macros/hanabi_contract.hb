use '../../../hanabi/json.hb' -> { Codec, parse, print }
use '../../../hanabi/view.hb' -> { render }
use '../../../hanabi/endpoint.hb' -> { Fault, request, parameters }

scalar Key(int[n -> n > 0])
record Item { id: Key, label: String }
union Action { Select(Key), Clear(unit) }
api { edit_api PUT '/items/{id}' (Key, Item) -> Item ! Fault = 200 }
fn main() {
    let item = Item { id: Key(3), label: '<script>' }
    put(print(item))
    put(print(Select(item.id)))
    put(match parse<Item>(print(item)) { Ok(v) -> v.label Err(e) -> e })
    put(render(<label for="entry"><input type="text" disabled={false} />{item.label}</label>))
    put(request(edit_api(), item.id, item).url)
    put(parameters('/items/{id}', '/items/4'))
}
