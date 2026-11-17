// 書き換えられる参照 Ref<T>。写し取った無名関数や with で提供する関数からも、同じ値を書き換えられる
trait Count { fn tick(self) }
fn work() with Count { tick()
    tick() }
fn main() {
    let n = new_ref(0)
    let log: Ref<Vec<String>> = new_ref([])
    let add = fn(s: String) { write(log, push(read(log), s)) }
    with Count(fn() { write(n, read(n) + 1) }) {
        work()
    }
    add('a')
    add('b')
    put(read(n))
    put(read(log))
    put(n == n)
}
