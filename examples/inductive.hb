// 右辺が | で区切った構築子の並びなら新しい型
type Nat = Zero | Succ(n: Nat)
type Option(T: Type) = None | Some(x: T)
// 添字付きの型族。Refl は a と a が等しいことの証明。構築子が1つなら先頭に | を書く
type Eq(A: Type, a: A, b: A) = | Refl -> Eq(A, a, a)

fn add(m: Nat, n: Nat) -> Nat {
    match m {
        Zero => n
        Succ(k) => Succ(add(k, n))
    }
}

// x = y なら Succ(x) = Succ(y)。p を Refl で場合分けすると y が x に置き換わる
fn cong(x: Nat, y: Nat, p: Eq(Nat, x, y)) -> Eq(Nat, Succ(x), Succ(y)) {
    match p { Refl => Refl }
}

// 0 + n = n は計算するだけで示せる
fn zero_add(n: Nat) -> Eq(Nat, add(Zero, n), n) { Refl }

// n + 0 = n は n についての帰納法で示す
fn add_zero(n: Nat) -> Eq(Nat, add(n, Zero), n) {
    match n {
        Zero => Refl
        Succ(k) => cong(add(k, Zero), k, add_zero(k))
    }
}

fn get(o: Option(int)) -> int {
    match o {
        None => 0
        Some(x) => x
    }
}

fn main() {
    let o = Some(3)
    print(get(o) + get(None))
    print(add(Succ(Zero), Succ(Succ(Zero))))
    print(add_zero(Succ(Zero)))
}
