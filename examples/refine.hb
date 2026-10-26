// 右辺が型の式なら別名。<> で型引数を渡せる
type NumVec(T: Type) = Vec<T>

// 細分化型: 条件を満たす値だけを許す型。条件はコンパイル時に証明する
type Pos = int[x -> x > 0]

// 添字 v[i] は 0 <= i && i < len(v) の証明が要る
fn sum(v: NumVec<Pos>) -> int {
    let mut s = 0
    let mut i: int[x -> x >= 0] = 0
    while i < len(v) {
        s = s + v[i]
        i = i + 1
    }
    s
}

fn main() {
    let v: NumVec<int[x -> x > 0]> = [3, 1, 4]
    print(v)
    print(sum(v))
    // let w: NumVec<Pos> = [1, 0] は 0 > 0 を証明できないので型エラー
}
