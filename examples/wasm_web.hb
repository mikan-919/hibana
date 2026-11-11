// ブラウザのホスト関数に接続するライブラリ。実行は wasm_web.html から行う。
extern "browser" {
    fn show(message: str)
    fn decorate(message: str) -> str
}

fn echo(message: str) -> str {
    message
}

fn answer(n: int) -> int {
    n + 1
}

fn greet(name: str) -> str {
    let message = decorate(name)
    show(message)
    message
}
