// 要求（with）は関数の型に載る。要求を持つ関数は、値として代入でき、引数にも渡せる。
// 値を呼ぶ時は、呼ぶ所を囲む with の提供を使う
trait Logger {
    fn log(self, msg: String)
}

struct SimpleLogger {}
impl Logger for SimpleLogger {
    fn log(self, msg: String) { put(msg) }
}

struct PrefixLogger { prefix: String }
impl Logger for PrefixLogger {
    fn log(self, msg: String) { put(self.prefix + msg) }
}

fn emit(msg: String)
with Logger(logger) {
    logger.log(msg)
}

// 要求を持つ関数型の引数。呼ぶ所を囲む with の提供を使う
fn apply(f: (String) -> unit with Logger, s: String)
with Logger(logger) {
    f(s)
}

fn each(xs: Vec<String>, f: (String) -> unit with Logger)
with Logger(logger) {
    for x in xs {
        f(x)
    }
}

fn main() {
    with Logger(SimpleLogger {}) {
        emit('direct')
        let f: (String) -> unit with Logger = emit
        f('as value')
        apply(emit, 'through a parameter')
        // 無名関数は、期待する型の要求を使える
        each(['a', 'b'], fn(s) { emit(s) })
        with Logger(PrefixLogger { prefix: '[inner] ' }) {
            apply(emit, 'inner')
        }
    }
}
