trait Logger {
    fn log(self, msg: String)
}

struct SimpleLogger {}
impl Logger for SimpleLogger {
    fn log(self, msg: String) { put(msg) }
}

fn make_logger() -> SimpleLogger {
    put('created')
    SimpleLogger {}
}

struct PrefixLogger { prefix: String }
impl Logger for PrefixLogger {
    fn log(self, msg: String) { put(self.prefix + msg) }
}

fn emit(msg: String)
with Logger(logger) {
    logger.log(msg)
}

fn forward(msg: String)
with Logger(logger) {
    emit(msg)
}

fn main() {
    with Logger(make_logger()) {
        forward('plain')
        with Logger(PrefixLogger { prefix: '[DEBUG] ' }) {
            forward('inner')
        }
        emit('restored')
    }
    with Logger(PrefixLogger { prefix: '[OTHER] ' }) {
        emit('other')
    }
}
