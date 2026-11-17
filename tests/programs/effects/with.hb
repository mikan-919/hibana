// 要求と提供（with）。関数は with で文脈から受け取るものを要求し、呼ぶ側は with ブロックで提供する
use std -> *

struct NotFound { id: String }
struct User { name: String }
struct Db { prefix: String }

trait Need<T> {
    fn need(self) -> T
}

trait Log {
    fn log(self, msg: String)
}

trait Logger {
    fn log(self, msg: String)
}

// 名前の無い要求は、メソッドを修飾なしで呼ぶ
fn find(id: String) -> User with Fail<NotFound>, Need<Db>, Log {
    let db = need()
    log('find ' + id)
    if id == '0' {
        fail(NotFound { id: id })
    } else {
        User { name: db.prefix + id }
    }
}

// impl を持つ値で提供する
struct Quiet {}

impl Log for Quiet {
    fn log(self, msg: String) {}
}

struct Prefix { text: String }

impl Logger for Prefix {
    fn log(self, msg: String) { put(self.text + msg) }
}

// 名前の有る要求は、名前.メソッド で呼ぶ。名前の無い Log と同じ名前のメソッドでも区別できる
fn both(msg: String) with Log, Logger(out) {
    log('quiet ' + msg)
    out.log('loud ' + msg)
}

// 要求の多相。R には要求と Async の両方が入る
fn twice<R: Effect>(f: () -> int with R) -> int with R {
    f() + f()
}

// メソッドの宣言に with Async があるトレイトは、提供する関数の中で待てる
trait Config {
    fn load(self) -> int with Async
}

fn fetch(n: int) -> int with Async {
    n * 10
}

fn configured() -> int with Config, Async {
    load().await + 1
}

fn lookup(id: String) -> String {
    // 無名関数で提供する。返すと呼んだ所が続き、exit すると with 全体の値になる
    with Need(fn() { Db { prefix: 'u' } }), Fail(fn(e) { exit 'not found: ' + e.id }), Log(Quiet {}) {
        find(id).name
    }
}

fn counted() -> int {
    let calls = new_ref(0)
    let n = with Log(fn(m) {
        write(calls, read(calls) + 1)
        put('log: ' + m)
    }) {
        twice(fn() {
            log('tick')
            3
        })
    }
    n + read(calls)
}

fn parse(s: String) -> int with Fail<String> {
    if s == 'x' { fail('bad: ' + s) } else { len(s) }
}

fn main() with Async {
    put(lookup('42'))
    put(lookup('0'))
    // 入れ子では内側が使われ、抜けると外側に戻る
    with Logger(Prefix { text: '[outer] ' }) {
        with Log(fn(m) { put('inner ' + m) }), Logger(Prefix { text: '[inner] ' }) {
            both('a')
        }
        with Log(Quiet {}) {
            both('b')
        }
    }
    put(counted())
    // exit の値は with の値。exit しなければ本体の値
    let early = with Fail(fn(e) { exit 0 - 1 }) {
        parse('x') + 100
    }
    put(early)
    // try は Fail を Result にする
    match try(fn() { parse('abc') }) {
        Ok(n) -> put('ok ' + n.to_string())
        Err(e) -> put('err ' + e)
    }
    match try(fn() { parse('x') }) {
        Ok(n) -> put('ok ' + n.to_string())
        Err(e) -> put('err ' + e)
    }
    // R に Async を通す。twice の呼び出しはその場で待つ
    put(twice(fn() { fetch(2).await }))
    // with Async を持つトレイトの提供では .await を使える
    put(with Config(fn() { fetch(4).await }) { configured().await })
}
