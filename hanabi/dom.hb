use std -> *
use './view.hb' -> { Html }
use './types.hb' -> { RequestId }

effect Dom {
    fn value(selector: String) -> String
    fn paint(selector: String, html: Html)
    fn focus(selector: String)
}
extern 'hanabi:browser' {
    fn read_value(selector: String) -> String
    fn render(selector: String, html: String)
    fn focus_element(selector: String)
    fn schedule(id: RequestId, method: String, url: String, body: String)
}
fn browser<A>(action: () -> A ! Dom) -> A {
    handle action() {
        value(selector) -> resume(read_value(selector))
        paint(selector, html) -> { render(selector, html.0); resume() }
        focus(selector) -> { focus_element(selector); resume() }
    }
}
