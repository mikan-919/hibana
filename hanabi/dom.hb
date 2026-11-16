// ブラウザとの接続。画面の状態と表示内容は hibana 側で決める
extern 'hanabi:browser' {
    fn render(selector: String, html: String)
    fn text(selector: String, text: String)
    fn value(selector: String) -> String
    fn set_value(selector: String, value: String)
    fn focus(selector: String)
    fn state(key: String) -> String
    fn set_state(key: String, value: String)
    fn request(method: String, path: String, body: String)
    fn busy(selector: String, value: int)
    fn notice(selector: String, message: String, error: int)
}
