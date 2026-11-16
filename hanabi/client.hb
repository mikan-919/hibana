use std -> *
use './json.hb' -> { ToJson }
use './view.hb' -> { Html }
use './dom.hb'

effect Remote { fn request(method: String, path: String, payload: Json) }
fn query(path: String) ! Remote { request('GET', path, JNull) }
fn send<A: ToJson>(method: String, path: String, value: A) ! Remote { request(method, path, value.to_json()) }
fn mount(selector: String, html: Html) { dom.render(selector, html.0) }
fn run(action: () -> unit ! Remote) {
    handle action() {
        request(method, path, payload) -> { dom.request(method, path, stringify(payload)); resume() }
    }
}
