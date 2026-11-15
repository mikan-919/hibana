// ブラウザのホスト関数に接続するライブラリ。実行は wasm_web.html から行う。
extern 'browser' {
    fn show(message: String)
    fn decorate(message: String) -> String
}

fn echo(message: String) -> String {
    message
}

fn answer(n: int) -> int {
    n + 1
}

fn greet(name: String) -> String {
    let message = decorate(name)
    show(message)
    message
}
