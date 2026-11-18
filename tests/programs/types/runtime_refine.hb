type Title = String[s -> 1 <= len(s) && len(s) <= 5]
type Positive = int[n -> n > 0]

fn title(s: String) -> Result<Title, String> { refine<Title>(s) }
fn once(counter: Ref<int>) -> String { write(counter, read(counter) + 1); 'ok' }
fn require_title(s: String) -> Title {
    match title(s) { Ok(value) -> value Err(message) -> panic(message) }
}
trait Factory { fn value() -> Self }
impl Factory for int { fn value() -> int { 3 } }
fn produce<A: Factory>() -> A { value() }

fn main() {
    put(refine<int>(3.0))
    put(refine<int>(1.5))
    put(refine<int>(9007199254740992.0))
    put(refine<int>(-9007199254740992.0))
    put(refine<int>(-0.0))
    put(refine<int[n -> n > 2]>(3.0))
    put(refine<int[n -> n > 2]>(2.0))
    for s in ['ok', '', 'length'] {
        put(match title(s) { Ok(value) -> 'ok:' + value Err(_) -> 'bad' })
    }
    let counter = new_ref(0)
    put(require_title(once(counter)))
    put(read(counter))
    put(match refine<Positive>(-1) { Ok(_) -> false Err(_) -> true })
    put(match refine<number[n -> n > 0 && n < 10]>(1.5) { Ok(v) -> v.to_string() Err(_) -> 'bad' })
    put(produce<int>())
    outer_variables()
}

// 条件に外側の変数を使える。値を検査する前に変数を写すので、値の式が mut の変数を書き換えても型を評価した時点の値で検査する
fn bound_check(limit: int, x: int) -> Result<int[n -> n > limit], String> { refine<int[n -> n > limit]>(x) }

fn outer_variables() {
    put(bound_check(3, 5))
    put(bound_check(3, 2))
    let mut m = 10
    let r = refine<int[n -> n < m]>({
        m = 0
        5
    })
    put(r)
    put(m)
    for lo in [1, 7] {
        put(match refine<int[n -> lo <= n && n < lo + 5]>(3) { Ok(v) -> v Err(_) -> -1 })
    }
}
