// ---- JSX: 要素を HTML の文字列にする ----
syntax JsxAttr = name:Ident "=" "{" value:Expr "}"
syntax JsxElement =
    "<" ~ tag:Ident attrs:JsxAttr* ">"
    children:JsxChild*
    "</" ~ close:Ident ~ ">"
syntax JsxChild =
    | el:JsxElement
    | "{" expr:Expr "}"
    | text:Text

// 子の要素はそのまま差し込み、展開結果の中で jsx がもう一度展開する
macro jsx (el:JsxElement) -> Expr {
    let tag = str_of(el.tag)
    if tag != str_of(el.close) {
        error("閉じタグ " + str_of(el.close) + " が " + tag + " と合わない")
    }
    let mut acc = quote { "<" + ${tag} }
    let mut i = 0
    while i < len(el.attrs) {
        let a = at(el.attrs, i)
        acc = quote { $acc + " " + ${str_of(a.name)} + "=\"" + to_str(${a.value}) + "\"" }
        i = i + 1
    }
    acc = quote { $acc + ">" }
    let mut j = 0
    while j < len(el.children) {
        let c = at(el.children, j)
        if !is_null(c.text) {
            acc = quote { $acc + ${str_of(c.text)} }
        } else if !is_null(c.expr) {
            acc = quote { $acc + to_str(${c.expr}) }
        } else {
            acc = quote { $acc + ${c.el} }
        }
        j = j + 1
    }
    quote { $acc + "</" + ${tag} + ">" }
}

// ---- routes!: ルートの並びを (メソッド, パス) -> 応答 の関数にする ----
syntax Annotation = "@" ~ name:Ident "(" args:ExprList? ")" "->" alias:Ident
syntax ExprList = items:(Expr % ",")
syntax Route = annotations:Annotation* method:Ident path:String body:Block

// パスの {名前} はその名前の変数になる。注釈 @f(引数) -> x は let x = f(引数)
macro routes! ("routes" ~ "!" "{" rs:Route* "}") -> Expr {
    let mut acc = quote { "404 Not Found" }
    let mut i = len(rs)
    while i > 0 {
        i = i - 1
        let r = at(rs, i)
        let segs = split(str_of(r.path), "/")
        let mut cond = quote { method == ${str_of(r.method)} && len(parts) == ${len(segs)} }
        let mut binds = quote { }
        let mut k: int[x -> x >= 0] = 0
        while k < len(segs) {
            let s = segs[k]
            if starts_with(s, "{") {
                binds = quote { $binds let ${ident(slice(s, 1, len(s) - 1))} = parts[${k}]; }
            } else {
                cond = quote { $cond && parts[${k}] == ${s} }
            }
            k = k + 1
        }
        let mut j = 0
        while j < len(r.annotations) {
            let a = at(r.annotations, j)
            binds = quote { $binds let ${a.alias} = ${a.name}(${a.args}); }
            j = j + 1
        }
        acc = quote { if $cond { $binds ${r.body} } else { $acc } }
    }
    quote { fn(method: str, path: str) -> str { let parts = split(path, "/"); $acc } }
}

fn durable_objects(name: str) -> str { "stub(" + name + ")" }

fn main() {
    let user = "Alice"
    let page = <div class={"card"}>
        Hello {user}!
        <Icon name={"star"}></Icon>
    </div>
    print(page)

    let app = routes! {
        GET "/user/{id}" {
            "user " + id
        }

        @durable_objects("ACCOUNT") -> account
        POST "/post/{user}/{post}" {
            account + ": post " + post + " by " + user
        }
    }
    print(app("GET", "/user/42"))
    print(app("POST", "/post/bob/7"))
    print(app("GET", "/nothing"))
}
