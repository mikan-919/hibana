macro Wrapped('wrapped' body:Block) {
    quote {
        let result = $body
        put(result)
        result + 1
    }
}

fn main() {
    let result = 10
    put(wrapped { 2 })
    put(result)
}
