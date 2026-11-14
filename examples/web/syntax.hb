// hanabi の構文。route { メソッド パス { 本体 } ... } と Response::json({ キー: 値 ... })
use web
use './lib' -> *

// 1970 年からのミリ秒
fn Clock::now() -> int { now_ms() }

syntax HanabiAnnotation = '@' ~ name:Ident '(' args:(Expr % ',') ')' '->' alias:Ident
syntax HanabiPath = | s:String | t:Template
syntax HanabiRoute = annotations:HanabiAnnotation* method:Ident path:HanabiPath body:Block

// 経路の並びを (web.Request) -> web.Response の関数にする。どれにも合わなければ web.not_found()
// パス '/{id}' の id と、クエリ `?time={time: 'day' | 'night'}` の time は Param の変数になる。| で並べた値以外のクエリは合わない
// 注釈 @durable_object('ACCOUNT') -> account は let account = durable_object('ACCOUNT') になる
macro route ('route' '{' rs:HanabiRoute* '}') -> Expr {
    // 空白と ' を除く（'day' を day に）
    let strip = fn(s: str) -> str {
        let mut out = ''
        for c in chars(s) {
            if c != ' ' && c != '\'' { out = out + c }
        }
        out
    }
    let mut acc = quote { web.not_found() }
    let mut i = len(rs)
    while i > 0 {
        i = i - 1
        let r = at(rs, i)
        let full = if is_null(r.path.s) { str_of(r.path.t) } else { str_of(r.path.s) }
        let mut path = ''
        let mut query = ''
        let mut n = 0
        for part in split(full, '?') {
            if n == 0 { path = part } else { query = part }
            n = n + 1
        }
        let segs = split(path, '/')
        let mut cond = quote { req.method == ${str_of(r.method)} && len(parts) == ${len(segs)} }
        // 本体から見える req（衛生性で隠れた引数の req を指す）
        let mut binds = quote { let ${ident('req')} = req; }
        let mut k = 0
        for s in segs {
            if starts_with(s, '{') {
                binds = quote { $binds let ${ident(slice(s, 1, len(s) - 1))} = Param::of(parts[${k}]); }
            } else {
                cond = quote { $cond && parts[${k}] == ${s} }
            }
            k = k + 1
        }
        if query != '' {
            for q in split(query, '&') {
                let mut key = ''
                let mut val = ''
                let mut m = 0
                for kv in split(q, '=') {
                    if m == 0 { key = kv } else { val = kv }
                    m = m + 1
                }
                if starts_with(val, '{') {
                    // {名前} か {名前: 'a' | 'b'}
                    let mut name = ''
                    let mut alts = ''
                    let mut j = 0
                    for p in split(slice(val, 1, len(val) - 1), ':') {
                        if j == 0 { name = strip(p) } else { alts = p }
                        j = j + 1
                    }
                    binds = quote { $binds let ${ident(name)} = Param::of(web.query(req, ${key})); }
                    if alts == '' {
                        cond = quote { $cond && web.query(req, ${key}) != '' }
                    } else {
                        let mut any = quote { false }
                        for a in split(alts, '|') {
                            any = quote { $any || web.query(req, ${key}) == ${strip(a)} }
                        }
                        cond = quote { $cond && ($any) }
                    }
                } else {
                    cond = quote { $cond && web.query(req, ${key}) == ${val} }
                }
            }
        }
        let mut j = 0
        while j < len(r.annotations) {
            let a = at(r.annotations, j)
            if str_of(a.name) == 'durable_object' {
                binds = quote { $binds let ${a.alias} = durable_object($( $a.args ),*); }
            } else {
                binds = quote { $binds let ${a.alias} = ${a.name}($( $a.args ),*); }
            }
            j = j + 1
        }
        acc = quote { if $cond { $binds ${r.body} } else { $acc } }
    }
    quote { fn(req: web.Request) -> web.Response { let parts = split(req.path, '/'); $acc } }
}

syntax JsonField = key:Ident ':' value:Expr

// Response::json({ status: 'OK' code: 200 }) は Response::json('{"status":"OK","code":200}')。値は Json の実装で文字列にする
macro response_json ('Response::json' '(' '{' fields:JsonField* '}' ')') -> Expr {
    let mut acc = quote { '{' }
    let mut i = 0
    while i < len(fields) {
        let f = at(fields, i)
        let head = if i == 0 { '"' } else { ',"' }
        acc = quote { $acc + ${head + str_of(f.key) + '":'} + to_json(${f.value}) }
        i = i + 1
    }
    quote { Response::json($acc + '}') }
}
