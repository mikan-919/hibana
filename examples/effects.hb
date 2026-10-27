// 代数的エフェクト。効果は effect で宣言し、関数の型の ! の後ろに書く。handle で処理すると型から消える

struct NotFound { id: str }
struct User { name: str, age: int }

// 失敗。resume しない腕は handle 全体の値になる
effect Fail<E> {
    fn fail(e: E) -> never
}

// 依存。腕で値を渡して resume する
effect Need<T> {
    fn need() -> T
}

effect Log {
    fn log(msg: str)
}

struct Db { prefix: str }

fn find(id: str) -> User ! Fail<NotFound>, Need<Db>, Log {
    let db = need()
    log("find " + id)
    if id == "0" {
        fail(NotFound { id: id })
    } else {
        User { name: db.prefix + id, age: 20 }
    }
}

fn describe_or(id: str) -> str {
    handle describe(id) {
        need() => resume(Db { prefix: "u" })
        log(msg) => resume()
    }
}

fn describe(id: str) -> str ! Need<Db>, Log {
    handle find(id).name {
        fail(e) => "not found: " + e.id
    }
}

// 関数型にも効果を書ける。効果の無い関数は、効果を持つ関数型の値として渡せる
fn each(xs: Vec<str>, f: (str) -> unit ! Log) ! Log {
    for x in xs { f(x) }
}

// all は Async の効果。インタプリタと C では順に実行する
fn main() ! Async {
    print(all([fn() { describe_or("7") }, fn() { describe_or("0") }]))
    handle each(["a", "b"], fn(x) { log("each " + x) }) {
        log(msg) => {
            print(msg)
            resume()
        }
    }
    let mut i = 0
    for id in ["1", "0", "2"] {
        let s = handle describe(id) {
            need() => resume(Db { prefix: "user" })
            log(msg) => {
                print("[log] " + msg)
                resume()
            }
        }
        print(s)
        i = i + 1
    }
    print(i)
}
