enum Step { Skip, Stop, Keep(int) }

trait Reporter {
    fn report(self, n: int)
}

fn main() {
    let mut i = 0
    while i < 4 {
        i = i + 1
        if i == 2 { continue }
        for j in 0..4 {
            if j == 1 { continue }
            if j == 3 { break }
            put(i * 10 + j)
        }
        if i == 3 { break }
    }

    for step in [Skip, Keep(4), Stop, Keep(5)] {
        match step {
            Skip -> continue,
            Stop -> break,
            Keep(n) -> put(n),
        }
    }

    for _ in 0..1 {
        while { break } { put(90) }
        put(91)
    }

    with Reporter(fn(n) { put(n) }) {
        for n in 0..3 {
            with Reporter(fn(n) { put(n + 100) }) {
                if n == 0 { continue }
                if n == 2 { break }
                report(n)
            }
            report(n)
        }
        report(9)
    }

    let mut limit = 3
    for n in 0..limit {
        limit = 0
        if n == 0 { continue }
        put(n)
    }

    for n in 0..9007199254740991 {
        put(n)
        break
    }
}
