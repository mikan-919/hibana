// 書き換えられる参照 Ref<T>。写し取った無名関数や handle の腕からも、同じ値を書き換えられる
effect Count { fn tick() }
fn work() ! Count { tick()
    tick() }
fn main() {
    let n = new_ref(0)
    let log: Ref<Vec<str>> = new_ref([])
    let add = fn(s: str) { write(log, push(read(log), s)) }
    handle work() {
        tick() => {
            write(n, read(n) + 1)
            resume()
        }
    }
    add("a")
    add("b")
    print(read(n))
    print(read(log))
    print(n == n)
}
