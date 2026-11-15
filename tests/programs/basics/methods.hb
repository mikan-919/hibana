// 型に属する関数（T::f）、メソッド呼び出し（x.f()）、埋め込み付きの文字列（`...{式}...`）
struct Param { value: str }

fn Param::get(p: Param) -> str { p.value }

fn Param::with(p: Param, s: str) -> Param { Param { value: p.value + s } }

fn str::twice(s: str) -> str { s + s }

fn main() {
    let id = Param { value: 'u7' }
    put(id.get())
    put(id.with('!').get().twice())
    put(Param::get(id))
    let n = 3
    put(`id={id.get()} n={n + 1} {if n == 3 { 'three' } else { `other {n}` }} \{x\}`)
}
