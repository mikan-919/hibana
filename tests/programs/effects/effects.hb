// 要求と提供。要求はトレイトで宣言し、関数の with に書く。with ブロックで提供すると型から消える
use std -> *

struct NotFound { id: String }
struct User { name: String, age: int }

// 依存。提供する関数の値を返すと、呼んだ所が続く
trait Need<T> {
    fn need(self) -> T
}

trait Log {
    fn log(self, msg: String)
}

struct Db { prefix: String }

// 失敗は std の Fail。exit した値は with 全体の値になる
fn find(id: String) -> User with Fail<NotFound>, Need<Db>, Log {
    let db = need()
    log('find ' + id)
    if id == '0' {
        fail(NotFound { id: id })
    } else {
        User { name: db.prefix + id, age: 20 }
    }
}

fn describe_or(id: String) -> String {
    with Need(fn() { Db { prefix: 'u' } }), Log(fn(msg) {}) {
        describe(id)
    }
}

fn describe(id: String) -> String with Need<Db>, Log {
    with Fail(fn(e) { exit 'not found: ' + e.id }) {
        find(id).name
    }
}

// 関数型にも要求を書ける。要求の無い関数は、要求を持つ関数型の値として渡せる
fn each(xs: Vec<String>, f: (String) -> unit with Log) with Log {
    for x in xs { f(x) }
}

// all は Async<T> の値を並行に走らせて合流する。インタプリタでは順に実行する
fn main() with Async {
    put(all([describe_later('7'), describe_later('0')]).await)
    with Log(fn(msg) { put(msg) }) {
        each(['a', 'b'], fn(x) { log('each ' + x) })
    }
    let mut i = 0
    for id in ['1', '0', '2'] {
        let s = with Need(fn() { Db { prefix: 'user' } }), Log(fn(msg) { put('[log] ' + msg) }) {
            describe(id)
        }
        put(s)
        i = i + 1
    }
    put(i)
}

fn describe_later(id: String) -> String with Async {
    describe_or(id)
}
