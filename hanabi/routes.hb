use './server.hb' -> { matches }

syntax Route = method:Ident path:String '->' body:Block
macro Routes('routes' request:Expr '{' entries:Route* '}') {
    let mut result = quote { server.error(404, 'ページが見つからない') }
    let mut n = len(entries)
    while n > 0 {
        n = n - 1
        let route = at(entries, n)
        let method = str_of(route.method)
        if method != 'GET' && method != 'POST' && method != 'PUT' && method != 'DELETE' { error('経路のメソッドが未対応: ' + method) }
        let mut k = 0
        while k < n {
            let previous = at(entries, k)
            if str_of(previous.method) == method && str_of(previous.path) == str_of(route.path) { error('経路が重複している: ' + method + ' ' + str_of(route.path)) }
            k = k + 1
        }
        result = quote { if matches(incoming, ${method}, $route.path) $route.body else { $result } }
    }
    quote { { let incoming = $request; $result } }
}
