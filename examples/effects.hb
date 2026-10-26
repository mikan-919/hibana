// 代数的エフェクト。効果は effect で宣言し、関数の型の ! の後ろに書く。handle で処理すると型から消える

type NotFound = { id: str }
type User = { name: str, age: int }

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

type Db = { prefix: str }

fn find(id: str) -> User ! Fail<NotFound>, Need<Db>, Log {
    let db = need()
    log("find " + id)
    if id == "0" {
        fail(NotFound { id: id })
    } else {
        User { name: db.prefix + id, age: 20 }
    }
}

fn describe(id: str) -> str ! Need<Db>, Log {
    handle find(id).name {
        fail(e) => "not found: " + e.id
    }
}

fn main() {
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
