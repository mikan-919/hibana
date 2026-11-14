fn fib(n: int) -> int {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

fn is_prime(n: int) -> bool {
    let mut i = 2
    while i * i <= n {
        if n % i == 0 { return false }
        i = i + 1
    }
    n >= 2
}

fn main() {
    let mut i = 0
    while i <= 10 {
        print(fib(i))
        i = i + 1
    }
    print(is_prime(97))
}
