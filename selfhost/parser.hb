// 構文解析器。src/parser.rs と同じ構文木を作る（S 式にして比べる）
// ponytail: マクロは derive(Eq, Show) だけを組み込みで展開し、syntax・macro・test の宣言は読み飛ばす。extern は無い。エラーは panic で止める
use "./lexer.hb".*
use std.*

// 式・文・ブロック・腕・引数を一つの型にする（相互に再帰する型を避ける）
enum Expr {
    EInt(n: int),
    EBool(b: bool),
    EStr(s: str),
    EVar(name: str),
    EUnary(op: str, x: Expr),
    EBinary(op: str, a: Expr, b: Expr),
    ECall(f: str, args: Vec<Expr>),
    EField(x: Expr, f: str),
    EMethod(x: Expr, f: str, args: Vec<Expr>),
    EAssign(name: str, x: Expr),
    EIf(c: Expr, then: Expr, els: Option<Expr>),
    EWhile(c: Expr, body: Expr),
    EBlock(stmts: Vec<Expr>, tail: Option<Expr>),
    ELet(name: str, mutable: bool, ty: Option<Expr>, init: Expr),
    EReturn(x: Option<Expr>),
    EArray(items: Vec<Expr>),
    EIndex(x: Expr, i: Expr),
    ERefine(t: Expr, x: str, c: Expr),
    EMatch(x: Expr, arms: Vec<Expr>),
    EArm(ctor: str, binds: Vec<str>, body: Expr),
    ELam(params: Vec<Expr>, ret: Option<Expr>, effs: Vec<Expr>, body: Expr),
    EPi(params: Vec<Expr>, ret: Expr, effs: Vec<Expr>),
    EApply(f: Expr, args: Vec<Expr>),
    EFor(x: str, a: Expr, b: Option<Expr>, body: Expr),
    // handle 式 { 操作(引数) => 式 ... }。腕は EArm（ctor は操作の名前）
    EHandle(x: Expr, arms: Vec<Expr>),
    // 引数 名前: 型（無名関数では型を省ける）
    EParam(name: str, ty: Option<Expr>),
}

struct Ctor { name: str, fields: Vec<Expr>, ret: Option<Expr> }
struct Data { name: str, params: Vec<Expr>, ctors: Vec<Ctor> }
// kind: "" は普通の関数、"trait" はトレイトのメソッド（本体が無い）、"impl" は実装のメソッド（名前は メソッド#型）、
// "default" はトレイトのメソッドの既定の実装（名前は メソッド#default）、"js:パス" は JS のモジュールの関数（名前は 別名.関数）、"op" は効果の操作（本体が無い。effs に効果の型が一つ入る）
struct Func { name: str, implicit: int, is_partial: bool, params: Vec<Expr>, ret: Option<Expr>, effs: Vec<Expr>, body: Expr, kind: str }
struct StructInfo { name: str, fields: Vec<str> }
// モジュールが宣言した名前。use したモジュールを読む時に使う
struct Ext { types: Vec<str>, generics: Vec<str>, structs: Vec<StructInfo> }

// 読んでいる位置と、前もって集めた名前
struct P {
    ts: Vec<Token>,
    pos: Ref<int>,
    // 宣言された型の名前。直後の < を型引数として読む
    types: Vec<str>,
    // ジェネリック引数を持つ関数の名前。f<int>(x) の < を型引数として読む
    generics: Vec<str>,
    structs: Vec<StructInfo>,
    // 戻り値の型を読んでいる間は、続く { を構造体の値と読まない
    no_struct: Ref<bool>,
}

// ---- トークンを読む ----

fn tok_at(p: P, k: int) -> Token {
    let i = read(p.pos) + k
    if i >= 0 && i < len(p.ts) { p.ts[i] } else { Token { tok: TEof, line: 0, col: 0, gap: false, nl: false } }
}

fn peek(p: P) -> Tok { tok_at(p, 0).tok }

// 今のトークンが前のトークンと別の行にあるか
fn nl(p: P) -> bool { tok_at(p, 0).nl }

fn advance(p: P) { write(p.pos, read(p.pos) + 1) }

fn is_p(t: Tok, s: str) -> bool {
    match t { TPunct(q) => q == s, _ => false }
}

fn is_ident(t: Tok) -> bool {
    match t { TIdent(n) => true, _ => false }
}

fn fail(p: P, msg: str) -> never {
    let t = tok_at(p, 0)
    panic(msg + "（" + to_str(t.line) + " 行 " + to_str(t.col) + " 列、実際は " + to_str(t.tok) + "）")
}

fn eat(p: P, s: str) -> bool {
    if is_p(peek(p), s) { advance(p); true } else { false }
}

fn expect(p: P, s: str) {
    if !eat(p, s) { fail(p, "\"" + s + "\" が必要") }
}

fn take_ident(p: P) -> str {
    match peek(p) {
        TIdent(n) => { advance(p); n }
        _ => fail(p, "識別子が必要"),
    }
}

// 文の区切り: ";"、改行、"}" の直前のいずれか
fn end_stmt(p: P) {
    if !(eat(p, ";") || nl(p) || is_p(peek(p), "}")) { fail(p, "文の区切り（\";\" か改行）が必要") }
}

fn struct_fields(p: P, name: str) -> Option<Vec<str>> {
    let mut r: Option<Vec<str>> = None
    for s in p.structs { if s.name == name { r = Some(s.fields) } }
    r
}

// ---- 式 ----

fn binop(t: Tok) -> int {
    match t {
        TPunct(p) => if p == "||" { 1 } else if p == "&&" { 2 } else if p == "==" || p == "!=" { 3 }
            else if p == "<" || p == "<=" || p == ">" || p == ">=" { 4 } else if p == "+" || p == "-" { 5 }
            else if p == "*" || p == "/" || p == "%" { 6 } else { 0 }
        _ => 0,
    }
}

partial fn expr(p: P) -> Expr {
    match peek(p) {
        TIdent(name) => if is_p(tok_at(p, 1).tok, "=") {
            advance(p)
            advance(p)
            EAssign(name, expr(p))
        } else { binary(p, 0) }
        _ => binary(p, 0),
    }
}

// 型は式として読む。"=" を代入と取り違えないよう二項演算から読む
partial fn ty(p: P) -> Expr { binary(p, 0) }

partial fn ret_ty(p: P) -> Expr {
    let saved = read(p.no_struct)
    write(p.no_struct, true)
    let t = ty(p)
    write(p.no_struct, saved)
    t
}

partial fn binary(p: P, min: int) -> Expr {
    let mut lhs = unary(p)
    let mut going = true
    while going {
        let prec = binop(peek(p))
        // 行頭の演算子は前の行に続けない（"-x" を新しい文として読むため）
        if prec == 0 || prec < min || nl(p) {
            going = false
        } else {
            let op = match peek(p) { TPunct(o) => o, _ => "" }
            advance(p)
            lhs = EBinary(op, lhs, binary(p, prec + 1))
        }
    }
    lhs
}

partial fn unary(p: P) -> Expr {
    if eat(p, "-") { EUnary("-", unary(p)) } else if eat(p, "!") { EUnary("!", unary(p)) } else { primary(p) }
}

// close までの , 区切りの式。開き括弧は読み終えた状態で呼ぶ
partial fn list_of(p: P, close: str) -> Vec<Expr> {
    let mut xs: Vec<Expr> = []
    let mut going = !eat(p, close)
    while going {
        xs = push(xs, expr(p))
        if !eat(p, ",") { expect(p, close); going = false } else if eat(p, close) { going = false }
    }
    xs
}

// <型, ...>。閉じの > を比較と読まないよう、加減乗除より弱い演算子は読まない
partial fn type_args(p: P) -> Vec<Expr> {
    let mut xs: Vec<Expr> = []
    let mut going = true
    while going {
        xs = push(xs, binary(p, 5))
        if !eat(p, ",") { expect(p, ">"); going = false }
    }
    xs
}

partial fn primary(p: P) -> Expr {
    let t = peek(p)
    advance(p)
    let e = match t {
        TInt(n) => EInt(n),
        TStr(s) => EStr(s),
        TIdent(name) => ident_expr(p, name),
        TPunct(k) => if k == "true" { EBool(true) } else if k == "false" { EBool(false) }
            else if k == "[" { EArray(list_of(p, "]")) }
            else if k == "(" { paren(p) }
            else if k == "fn" { lambda(p) }
            else if k == "{" { block(p) }
            else if k == "if" { if_rest(p) }
            else if k == "while" { let c = expr(p); expect(p, "{"); EWhile(c, block(p)) }
            else if k == "for" { for_rest(p) }
            else if k == "match" { match_rest(p) }
            else if k == "handle" { handle_rest(p) }
            else if k == "return" {
                if nl(p) || is_p(peek(p), ";") || is_p(peek(p), "}") { EReturn(None) } else { EReturn(Some(expr(p))) }
            }
            else { write(p.pos, read(p.pos) - 1); fail(p, "式が必要") }
        _ => { write(p.pos, read(p.pos) - 1); fail(p, "式が必要") }
    }
    postfix(p, e)
}

partial fn ident_expr(p: P, name: str) -> Expr {
    let fields = struct_fields(p, name)
    let tuple = match fields { Some(fs) => len(fs) > 0 && fs[0] == "0", None => false }
    if tuple && !nl(p) && is_p(peek(p), "(") {
        // struct Name(型, ...) の値 Name(式, ...)。構築子 Name{} の呼び出しにする
        advance(p)
        ECall(name + "{}", list_of(p, ")"))
    } else if contains(p.types, name) && !nl(p) && is_p(peek(p), "(") {
        fail(p, "型 " + name + " には <> で引数を渡す")
    } else if is_some(fields) && !read(p.no_struct) && !nl(p) && is_p(peek(p), "{") {
        advance(p)
        struct_lit(p, name, unwrap_or(fields, []))
    } else if contains(p.types, name) && is_p(peek(p), "<") {
        advance(p)
        ECall(name, type_args(p))
    } else if contains(p.generics, name) && is_p(peek(p), "<") {
        // f<型, ...>(引数) は型引数を引数の先頭に並べる
        advance(p)
        let targs = type_args(p)
        expect(p, "(")
        ECall(name, concat(targs, list_of(p, ")")))
    } else if !nl(p) && eat(p, "(") {
        ECall(name, list_of(p, ")"))
    } else {
        EVar(name)
    }
}

fn is_some(o: Option<Vec<str>>) -> bool { match o { Some(x) => true, None => false } }

fn concat(a: Vec<Expr>, b: Vec<Expr>) -> Vec<Expr> {
    let mut r = a
    for x in b { r = push(r, x) }
    r
}

// 構造体の値 Name { フィールド: 式, ... }。宣言の順に並べた構築子 Name{} の呼び出しにする
partial fn struct_lit(p: P, name: str, order: Vec<str>) -> Expr {
    let mut names: Vec<str> = []
    let mut vals: Vec<Expr> = []
    while !eat(p, "}") {
        names = push(names, take_ident(p))
        expect(p, ":")
        vals = push(vals, expr(p))
        eat(p, ",")
    }
    let mut args: Vec<Expr> = []
    for f in order {
        let mut found = false
        for i in 0..len(names) {
            if names[i] == f && i < len(vals) { args = push(args, vals[i]); found = true }
        }
        if !found { fail(p, "構造体 " + name + " のフィールド " + f + " が無い") }
    }
    for n in names { if !contains(order, n) { fail(p, "構造体 " + name + " にフィールド " + n + " は無い") } }
    ECall(name + "{}", args)
}

// (x: A, B) -> C は関数型。名前の無い1個だけなら括弧で囲んだ式
partial fn paren(p: P) -> Expr {
    let mut entries: Vec<Expr> = []
    let mut named = false
    let mut going = !eat(p, ")")
    while going {
        let mut name = "_"
        if is_ident(peek(p)) && is_p(tok_at(p, 1).tok, ":") {
            name = take_ident(p)
            advance(p)
            named = true
        }
        entries = push(entries, EParam(name, Some(expr(p))))
        if !eat(p, ",") { expect(p, ")"); going = false } else if eat(p, ")") { going = false }
    }
    if eat(p, "->") {
        let ret = binary(p, 5)
        EPi(entries, ret, effects(p))
    } else if len(entries) == 1 && !named {
        match entries[0] { EParam(n, t) => unwrap_or(t, EVar("_")), _ => entries[0] }
    } else {
        fail(p, "関数型なら ) の後に -> が必要")
    }
}

partial fn lambda(p: P) -> Expr {
    expect(p, "(")
    let mut ps: Vec<Expr> = []
    let mut going = !eat(p, ")")
    while going {
        let n = take_ident(p)
        ps = push(ps, EParam(n, if eat(p, ":") { Some(ty(p)) } else { None }))
        if !eat(p, ",") { expect(p, ")"); going = false } else if eat(p, ")") { going = false }
    }
    let ret = if eat(p, "->") { Some(ret_ty(p)) } else { None }
    let effs = effects(p)
    expect(p, "{")
    ELam(ps, ret, effs, block(p))
}

partial fn if_rest(p: P) -> Expr {
    let c = expr(p)
    expect(p, "{")
    let then = block(p)
    let els = if !eat(p, "else") { None } else if eat(p, "if") { Some(if_rest(p)) } else { expect(p, "{"); Some(block(p)) }
    EIf(c, then, els)
}

partial fn for_rest(p: P) -> Expr {
    let x = take_ident(p)
    expect(p, "in")
    let a = expr(p)
    let b = if eat(p, "..") { Some(expr(p)) } else { None }
    expect(p, "{")
    EFor(x, a, b, block(p))
}

partial fn match_rest(p: P) -> Expr {
    let s = expr(p)
    expect(p, "{")
    let mut arms: Vec<Expr> = []
    while !eat(p, "}") {
        let ctor = take_ident(p)
        let mut binds: Vec<str> = []
        if eat(p, "(") {
            let mut going = !eat(p, ")")
            while going {
                binds = push(binds, take_ident(p))
                if !eat(p, ",") { expect(p, ")"); going = false } else if eat(p, ")") { going = false }
            }
        }
        expect(p, "=>")
        arms = push(arms, EArm(ctor, binds, expr(p)))
        eat(p, ",")
    }
    EMatch(s, arms)
}

partial fn handle_rest(p: P) -> Expr {
    let body = expr(p)
    expect(p, "{")
    let mut arms: Vec<Expr> = []
    while !eat(p, "}") {
        let op = take_ident(p)
        expect(p, "(")
        let mut binds: Vec<str> = []
        let mut going = !eat(p, ")")
        while going {
            binds = push(binds, take_ident(p))
            if !eat(p, ",") { expect(p, ")"); going = false } else if eat(p, ")") { going = false }
        }
        expect(p, "=>")
        arms = push(arms, EArm(op, binds, expr(p)))
        eat(p, ",")
    }
    EHandle(body, arms)
}

// 式に続く .名前、.名前(引数)、(引数)、[添字]、[x -> 条件]
partial fn postfix(p: P, e0: Expr) -> Expr {
    let mut e = e0
    let mut going = true
    while going && !nl(p) {
        if eat(p, ".") {
            // struct Name(型, ...) のフィールドは x.0
            let name = match peek(p) { TInt(k) => { advance(p); to_str(k) }, _ => take_ident(p) }
            e = if !nl(p) && eat(p, "(") { EMethod(e, name, list_of(p, ")")) } else { EField(e, name) }
        } else if eat(p, "(") {
            e = EApply(e, list_of(p, ")"))
        } else if eat(p, "[") {
            e = match peek(p) {
                TIdent(x) => if is_p(tok_at(p, 1).tok, "->") { advance(p); advance(p); ERefine(e, x, expr(p)) } else { EIndex(e, expr(p)) }
                _ => EIndex(e, expr(p)),
            }
            expect(p, "]")
        } else {
            going = false
        }
    }
    e
}

// "{" は読み終えた状態で呼ぶ
partial fn block(p: P) -> Expr {
    let mut stmts: Vec<Expr> = []
    let mut tail: Option<Expr> = None
    let mut going = true
    while going {
        if eat(p, "}") {
            going = false
        } else if is_p(peek(p), "let") {
            stmts = push(stmts, let_stmt(p))
        } else {
            let e = expr(p)
            if eat(p, ";") {
                stmts = push(stmts, e)
            } else if eat(p, "}") {
                tail = Some(e)
                going = false
            } else {
                end_stmt(p)
                stmts = push(stmts, e)
            }
        }
    }
    EBlock(stmts, tail)
}

partial fn let_stmt(p: P) -> Expr {
    expect(p, "let")
    let m = eat(p, "mut")
    let name = take_ident(p)
    let t = if eat(p, ":") { Some(ty(p)) } else { None }
    expect(p, "=")
    let init = expr(p)
    end_stmt(p)
    ELet(name, m, t, init)
}

// ! 効果, ... 。, の後が 名前: なら次の引数
partial fn effects(p: P) -> Vec<Expr> {
    let mut es: Vec<Expr> = []
    if eat(p, "!") {
        let mut going = true
        while going {
            let saved = read(p.no_struct)
            write(p.no_struct, true)
            es = push(es, binary(p, 5))
            write(p.no_struct, saved)
            let next_param = is_ident(tok_at(p, 1).tok) && is_p(tok_at(p, 2).tok, ":")
            if next_param || !eat(p, ",") { going = false }
        }
    }
    es
}

// ---- 宣言 ----

// ( は読み終えた状態で呼ぶ。名前: 型 の並び
partial fn params(p: P) -> Vec<Expr> {
    let mut ps: Vec<Expr> = []
    let mut going = !eat(p, ")")
    while going {
        let n = take_ident(p)
        expect(p, ":")
        ps = push(ps, EParam(n, Some(ty(p))))
        if !eat(p, ",") { expect(p, ")"); going = false } else if eat(p, ")") { going = false }
    }
    ps
}

// <名前: 型, ...>。型を省いた <A> は <A: Type>
partial fn type_params(p: P) -> Vec<Expr> {
    let mut ps: Vec<Expr> = []
    if eat(p, "<") {
        let mut going = true
        while going {
            let n = take_ident(p)
            ps = push(ps, EParam(n, Some(if eat(p, ":") { binary(p, 5) } else { EVar("Type") })))
            if !eat(p, ",") { expect(p, ">"); going = false }
        }
    }
    ps
}

partial fn func(p: P) -> Func {
    let part = eat(p, "partial")
    expect(p, "fn")
    let name = take_ident(p)
    let tps = type_params(p)
    expect(p, "(")
    let ps = concat(tps, params(p))
    let ret = if eat(p, "->") { Some(ret_ty(p)) } else { None }
    let effs = effects(p)
    expect(p, "{")
    Func { name: name, implicit: len(tps), is_partial: part, params: ps, ret: ret, effs: effs, body: block(p), kind: "" }
}

// struct・enum は Data、type は別名の関数
partial fn type_decl(p: P) -> Result<Data, Func> {
    let kw = match peek(p) { TPunct(k) => k, _ => "" }
    advance(p)
    let name = take_ident(p)
    let ps = type_params(p)
    if kw == "enum" {
        expect(p, "{")
        let mut cs: Vec<Ctor> = []
        while !eat(p, "}") {
            let c = take_ident(p)
            let fs = if eat(p, "(") { params(p) } else { [] }
            let ret = if eat(p, "->") { Some(ty(p)) } else { None }
            cs = push(cs, Ctor { name: c, fields: fs, ret: ret })
            eat(p, ",")
        }
        Ok(Data { name: name, params: ps, ctors: cs })
    } else if kw == "struct" {
        let tuple = eat(p, "(")
        if !tuple { expect(p, "{") }
        let close = if tuple { ")" } else { "}" }
        let mut fs: Vec<Expr> = []
        while !eat(p, close) {
            if tuple {
                fs = push(fs, EParam(to_str(len(fs)), Some(ty(p))))
            } else {
                let n = take_ident(p)
                expect(p, ":")
                fs = push(fs, EParam(n, Some(ty(p))))
            }
            eat(p, ",")
        }
        Ok(Data { name: name, params: ps, ctors: [Ctor { name: name + "{}", fields: fs, ret: None }] })
    } else {
        expect(p, "=")
        let e = ty(p)
        Err(Func { name: name, implicit: 0, is_partial: false, params: ps, ret: Some(EVar("Type")), effs: [], body: EBlock([], Some(e)), kind: "" })
    }
}

// 関数の本体の終わりまで読み飛ばす
fn skip_fn(p: P) {
    while !eat(p, "{") {
        if peek(p) == TEof { fail(p, "関数の本体が無い") }
        advance(p)
    }
    let mut depth = 1
    while depth > 0 {
        let t = peek(p)
        advance(p)
        if is_p(t, "{") { depth = depth + 1 } else if is_p(t, "}") { depth = depth - 1 } else if t == TEof { fail(p, "\"}\" が足りない") }
    }
}

fn is_type_kw(t: Tok) -> bool { is_p(t, "struct") || is_p(t, "enum") || is_p(t, "type") }

// 宣言の頭の語（derive・test は識別子）
fn head(p: P) -> str { match peek(p) { TPunct(k) => k, TIdent(n) => n, _ => "" } }

// 次の行の頭まで読み飛ばす（use）。cont で始まる行は続きとみなす（syntax の | の行）
fn skip_line(p: P, cont: str) {
    advance(p)
    while peek(p) != TEof && !(nl(p) && !is_p(peek(p), cont)) { advance(p) }
}

// use の対象。use "./x.hb".* なら "./x.hb"、use std.* なら "std"
fn uses_of(ts: Vec<Token>) -> Vec<str> {
    let mut r: Vec<str> = []
    let mut depth = 0
    for i in 0..len(ts) {
        let t = ts[i].tok
        if is_p(t, "{") || is_p(t, "(") || is_p(t, "[") { depth = depth + 1 }
        if is_p(t, "}") || is_p(t, ")") || is_p(t, "]") { depth = depth - 1 }
        if depth == 0 && is_p(t, "use") && i + 1 < len(ts) {
            match ts[i + 1].tok { TStr(s) => { r = push(r, s) }, TIdent(n) => { r = push(r, n) }, _ => {} }
        }
    }
    r
}

fn concat_f(a: Vec<Func>, b: Vec<Func>) -> Vec<Func> {
    let mut r = a
    for x in b { r = push(r, x) }
    r
}

// trait Name<A>: 親<A> { fn メソッド(引数) -> 型 ... }。メソッドはトレイトの型引数をジェネリック引数に持つ関数にする
// 既定の実装は メソッド#default という kind "default" の関数にする
// ponytail: 親のトレイトの検査は無い
partial fn trait_decl(p: P) -> Vec<Func> {
    expect(p, "trait")
    let tname = take_ident(p)
    let tps = type_params(p)
    while !eat(p, "{") { advance(p) }
    let mut fs: Vec<Func> = []
    while !eat(p, "}") {
        expect(p, "fn")
        let m = take_ident(p)
        expect(p, "(")
        let ps = params(p)
        let ret = if eat(p, "->") { Some(ret_ty(p)) } else { None }
        let effs = effects(p)
        fs = push(fs, Func { name: m, implicit: len(tps), is_partial: false, params: concat(tps, ps), ret: ret, effs: effs, body: EBlock([], None), kind: "trait" })
        if eat(p, "{") { fs = push(fs, Func { name: m + "#default", implicit: len(tps), is_partial: true, params: concat(tps, ps), ret: ret, effs: effs, body: block(p), kind: "default" }) }
        eat(p, ",")
    }
    // トレイトの名前とメソッド名。<A: Trait> の制約を検査するのに使う（実行時には出さない）
    let mut ms: Vec<Expr> = []
    for f in fs { if f.kind == "trait" { ms = push(ms, EParam(f.name, None)) } }
    push(fs, Func { name: tname, implicit: 0, is_partial: true, params: ms, ret: None, effs: [], body: EBlock([], None), kind: "traitdef" })
}

// effect Name<A> { fn 操作(引数) -> 型 ... }。効果は構築子の無い型、操作は効果の型引数をジェネリック引数に持つ関数
struct Effect { data: Data, ops: Vec<Func> }

partial fn effect_decl(p: P) -> Effect {
    expect(p, "effect")
    let name = take_ident(p)
    let tps = type_params(p)
    expect(p, "{")
    let me = if len(tps) == 0 { EVar(name) } else { ECall(name, map(tps, fn(t: Expr) -> Expr { match t { EParam(n, x) => EVar(n), _ => t } })) }
    let mut ops: Vec<Func> = []
    while !eat(p, "}") {
        expect(p, "fn")
        let m = take_ident(p)
        expect(p, "(")
        let ps = params(p)
        let ret = if eat(p, "->") { Some(ret_ty(p)) } else { None }
        ops = push(ops, Func { name: m, implicit: len(tps), is_partial: true, params: concat(tps, ps), ret: ret, effs: [me], body: EBlock([], None), kind: "op" })
        eat(p, ",")
    }
    Effect { data: Data { name: name, params: tps, ctors: [] }, ops: ops }
}

// 型の式の頭の名前。Vec<A> なら Vec
fn type_key(e: Expr) -> str { match e { EVar(n) => n, ECall(n, xs) => n, _ => "" } }

// impl<G> Trait<型> { fn メソッド(...) { ... } }。メソッドを メソッド#型 という名前の関数にする
partial fn impl_decl(p: P) -> Vec<Func> {
    expect(p, "impl")
    let tps = type_params(p)
    let key = match ret_ty(p) { ECall(t, xs) => if len(xs) == 1 { type_key(xs[0]) } else { "" }, _ => "" }
    if key == "" { fail(p, "impl の後には トレイト<型> が必要") }
    expect(p, "{")
    let mut fs: Vec<Func> = []
    while !eat(p, "}") {
        let f = func(p)
        fs = push(fs, Func { name: f.name + "#" + key, implicit: len(tps) + f.implicit, is_partial: f.is_partial, params: concat(tps, f.params), ret: f.ret, effs: f.effs, body: f.body, kind: "impl" })
    }
    fs
}

// derive(Eq, Show) の型の impl の hibana のコード（lib/std.hb の macro derive と同じ形）
fn derive_src(d: Data, traits: Vec<str>) -> str {
    let names = map(d.params, fn(e: Expr) -> str { match e { EParam(n, t) => n, _ => "" } })
    let gen = if len(names) == 0 { "" } else { "<" + join_s(names, ", ") + ">" }
    let ty = d.name + gen
    let mut eq = "true"
    let mut show = ""
    if len(d.ctors) == 1 && d.ctors[0].name == d.name + "{}" {
        show = "\"" + d.name + " {\""
        let fs = d.ctors[0].fields
        for k in 0..len(fs) {
            let f = match fs[k] { EParam(n, t) => n, _ => "" }
            eq = eq + " && a." + f + " == b." + f
            show = show + " + \"" + (if k == 0 { " " } else { ", " }) + f + ": \" + to_str(a." + f + ")"
        }
        show = show + " + \" }\""
    } else {
        let mut eq_arms = ""
        let mut show_arms = ""
        for c in d.ctors {
            if len(c.fields) == 0 {
                eq_arms = eq_arms + c.name + " => match b { " + c.name + " => true, _ => false }, "
                show_arms = show_arms + c.name + " => \"" + c.name + "\", "
            } else {
                let mut xs: Vec<str> = []
                let mut ys: Vec<str> = []
                let mut cond = "true"
                let mut s = "\"" + c.name + "(\""
                for j in 0..len(c.fields) {
                    let x = "x" + to_str(j)
                    let y = "y" + to_str(j)
                    xs = push(xs, x)
                    ys = push(ys, y)
                    cond = cond + " && " + x + " == " + y
                    s = s + (if j == 0 { "" } else { " + \", \"" }) + " + to_str(" + x + ")"
                }
                eq_arms = eq_arms + c.name + "(" + join_s(xs, ", ") + ") => match b { " + c.name + "(" + join_s(ys, ", ") + ") => " + cond + ", _ => false }, "
                show_arms = show_arms + c.name + "(" + join_s(xs, ", ") + ") => " + s + " + \")\", "
            }
        }
        eq = "match a { " + eq_arms + "}"
        show = "match a { " + show_arms + "}"
    }
    let mut out = ""
    for tr in traits {
        if tr == "Eq" { out = out + "impl" + gen + " Eq<" + ty + "> { fn eq(a: " + ty + ", b: " + ty + ") -> bool { " + eq + " } }\n" }
        else if tr == "Show" { out = out + "impl" + gen + " Show<" + ty + "> { fn show(a: " + ty + ") -> str { " + show + " } }\n" }
        else { panic("derive できるのは Eq と Show（" + tr + " は無い）") }
    }
    out
}

fn join_s(xs: Vec<str>, sep: str) -> str {
    let mut s = ""
    for i in 0..len(xs) { s = s + (if i > 0 { sep } else { "" }) + xs[i] }
    s
}

// derive(名前, ...) を読み、名前の並びを返す
fn derive_head(p: P) -> Vec<str> {
    advance(p)
    expect(p, "(")
    let mut r: Vec<str> = []
    while !eat(p, ")") { r = push(r, take_ident(p)); eat(p, ",") }
    r
}

struct Program { datas: Vec<Data>, fns: Vec<Func>, uses: Vec<str>, info: Ext }

fn no_ext() -> Ext { Ext { types: [], generics: [], structs: [] } }

partial fn parse(src: str) -> Program { parse_with(src, no_ext()) }

fn concat_str(a: Vec<str>, b: Vec<str>) -> Vec<str> {
    let mut r = a
    for x in b { r = push(r, x) }
    r
}

// ext は use したモジュールが宣言した名前
partial fn parse_with(src: str, ext: Ext) -> Program {
    match lex(src) { Ok(ts) => parse_tokens(ts, ext), Err(e) => panic(e.msg) }
}

partial fn parse_tokens(ts0: Vec<Token>, ext: Ext) -> Program {
    // 型の名前とジェネリックな関数の名前を前もって集める（括弧の外の宣言の頭だけを見る）
    let mut types: Vec<str> = []
    let mut generics: Vec<str> = []
    let mut depth = 0
    for i in 0..len(ts0) {
        let t = ts0[i].tok
        if is_p(t, "{") || is_p(t, "(") || is_p(t, "[") { depth = depth + 1 }
        if is_p(t, "}") || is_p(t, ")") || is_p(t, "]") { depth = depth - 1 }
        if depth == 0 && i + 2 < len(ts0) {
            match ts0[i + 1].tok {
                TIdent(n) => {
                    if is_type_kw(t) || is_p(t, "trait") || is_p(t, "effect") { types = push(types, n) }
                    if is_p(t, "fn") && is_p(ts0[i + 2].tok, "<") { generics = push(generics, n) }
                }
                _ => {}
            }
        }
    }
    let all_types = concat_str(concat_str(["Vec", "Ref", "Type"], types), ext.types)
    let all_generics = concat_str(generics, ext.generics)
    // 型の宣言を先に読み、構造体のフィールドの順をそろえてから関数を読む
    let p0 = P { ts: ts0, pos: new_ref(0), types: all_types, generics: all_generics, structs: [], no_struct: new_ref(false) }
    let mut datas: Vec<Data> = []
    let mut aliases: Vec<Func> = []
    let mut derived = ""
    while peek(p0) != TEof {
        let h = head(p0)
        let traits = if h == "derive" { derive_head(p0) } else { [] }
        if is_type_kw(peek(p0)) {
            match type_decl(p0) {
                Ok(d) => { datas = push(datas, d); if len(traits) > 0 { derived = derived + derive_src(d, traits) } }
                Err(f) => { aliases = push(aliases, f) }
            }
        } else if h == "effect" {
            datas = push(datas, effect_decl(p0).data)
        } else if h == "fn" || h == "partial" || h == "trait" || h == "impl" || h == "macro" || h == "test" {
            skip_fn(p0)
        } else if h == "use" {
            skip_line(p0, "")
        } else if h == "syntax" {
            skip_line(p0, "|")
        } else {
            fail(p0, "宣言が必要")
        }
    }
    // derive の impl はトークンにして末尾に足す
    let mut ts = filter(ts0, fn(t: Token) -> bool { t.tok != TEof })
    match lex(derived) { Ok(xs) => { for t in xs { ts = push(ts, t) } }, Err(e) => panic(e.msg) }
    let mut own: Vec<StructInfo> = []
    for d in datas {
        for c in d.ctors {
            if c.name == d.name + "{}" {
                own = push(own, StructInfo { name: d.name, fields: map(c.fields, fn(f: Expr) -> str { match f { EParam(n, t) => n, _ => "" } }) })
            }
        }
    }
    let mut structs = own
    for s in ext.structs { structs = push(structs, s) }
    let p = P { ts: ts, pos: new_ref(0), types: all_types, generics: all_generics, structs: structs, no_struct: new_ref(false) }
    let mut fns = aliases
    while peek(p) != TEof {
        let h = head(p)
        if h == "derive" { derive_head(p); }
        else if is_type_kw(peek(p)) { type_decl(p); }
        else if h == "fn" || h == "partial" { fns = push(fns, func(p)) }
        else if h == "trait" { fns = concat_f(fns, trait_decl(p)) }
        else if h == "effect" { fns = concat_f(fns, effect_decl(p).ops) }
        else if h == "impl" { fns = concat_f(fns, impl_decl(p)) }
        else if h == "use" { skip_line(p, "") }
        else if h == "syntax" { skip_line(p, "|") }
        else { skip_fn(p) }
    }
    Program { datas: datas, fns: fns, uses: uses_of(ts0), info: Ext { types: types, generics: generics, structs: own } }
}

// ---- S 式（src/parser.rs の sexp と同じ形） ----

fn list(head: str, xs: Vec<str>) -> str {
    let mut s = "(" + head
    for x in xs { s = s + " " + x }
    s + ")"
}

fn dquote(s: str) -> str {
    let mut r = "\""
    for c in chars(s) {
        r = r + (if c == "\"" { "\\\"" } else if c == "\\" { "\\\\" } else if c == "\n" { "\\n" } else if c == "\t" { "\\t" } else if c == "\r" { "\\r" } else { c })
    }
    r + "\""
}

partial fn opt(o: Option<Expr>) -> str {
    match o { None => "_", Some(e) => sexp(e) }
}

partial fn sexps(xs: Vec<Expr>) -> Vec<str> { map(xs, fn(x: Expr) -> str { sexp(x) }) }

partial fn sexp(e: Expr) -> str {
    match e {
        EInt(n) => to_str(n),
        EBool(b) => to_str(b),
        EStr(s) => dquote(s),
        EVar(n) => n,
        EUnary(op, x) => list(op, [sexp(x)]),
        EBinary(op, a, b) => list(op, [sexp(a), sexp(b)]),
        ECall(f, xs) => list("call " + f, sexps(xs)),
        EField(x, f) => list("field", [sexp(x), f]),
        EMethod(x, f, xs) => list("method", concat_s([sexp(x), f], sexps(xs))),
        EAssign(n, x) => list("assign", [n, sexp(x)]),
        EIf(c, t, els) => list("if", [sexp(c), sexp(t), opt(els)]),
        EWhile(c, b) => list("while", [sexp(c), sexp(b)]),
        EBlock(st, tail) => list("block", push(sexps(st), opt(tail))),
        ELet(n, m, t, init) => list(if m { "let-mut" } else { "let" }, [n, opt(t), sexp(init)]),
        EReturn(x) => list("return", [opt(x)]),
        EArray(xs) => list("array", sexps(xs)),
        EIndex(a, i) => list("index", [sexp(a), sexp(i)]),
        ERefine(t, x, c) => list("refine", [sexp(t), x, sexp(c)]),
        EMatch(x, arms) => list("match", concat_s([sexp(x)], sexps(arms))),
        EArm(c, bs, body) => list("arm", [c, list("binds", bs), sexp(body)]),
        ELam(ps, ret, effs, body) => list("lam", [list("params", sexps(ps)), opt(ret), list("effects", sexps(effs)), sexp(body)]),
        EPi(ps, ret, effs) => list("pi", [list("params", sexps(ps)), sexp(ret), list("effects", sexps(effs))]),
        EApply(f, xs) => list("apply", concat_s([sexp(f)], sexps(xs))),
        EFor(x, a, b, body) => list("for", [x, sexp(a), opt(b), sexp(body)]),
        EParam(n, t) => "(" + n + " " + opt(t) + ")",
        EHandle(x, arms) => list("handle", concat_s([sexp(x)], sexps(arms))),
    }
}

fn concat_s(a: Vec<str>, b: Vec<str>) -> Vec<str> {
    let mut r = a
    for x in b { r = push(r, x) }
    r
}

partial fn program(prog: Program) -> str {
    let mut out = ""
    for d in prog.datas {
        let cs = map(d.ctors, fn(c: Ctor) -> str { list("ctor " + c.name, [list("params", sexps(c.fields)), opt(c.ret)]) })
        out = out + list("data " + d.name, concat_s([list("params", sexps(d.params))], cs)) + "\n"
    }
    for f in prog.fns {
        let head = "fn " + f.name + " " + to_str(f.implicit) + " " + to_str(f.is_partial)
        out = out + list(head, [list("params", sexps(f.params)), opt(f.ret), list("effects", sexps(f.effs)), sexp(f.body)]) + "\n"
    }
    out
}

test "expressions" {
    assert_eq(program(parse("fn f(x: int) -> int { let y = -x * 2 + 1\n y }")),
        "(fn f 0 false (params (x int)) int (effects) (block (let y _ (+ (* (- x) 2) 1)) y))\n")
}

test "structs" {
    assert_eq(program(parse("struct P { x: int, y: int }\nfn f() -> P { P { y: 2, x: 1 } }")),
        "(data P (params) (ctor P{} (params (x int) (y int)) _))\n(fn f 0 false (params) P (effects) (block (call P{} 1 2)))\n")
}
