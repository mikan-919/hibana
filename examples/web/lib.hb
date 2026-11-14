// hanabi: Cloudflare Workers 向けの Web の枠組み。経路の構文は syntax.hb の route
use web

type Response = web.Response

// 経路の変数（/{id} の id、?time={time} の time）の値
struct Param { value: str }

fn Param::of(value: str) -> Param { Param { value: value } }

fn Param::get(p: Param) -> str { p.value }

fn Response::text(body: str) -> web.Response { web.ok(body) }

// body は JSON の文字列。Response::json({ キー: 値 ... }) の形は syntax.hb のマクロが JSON の文字列にする
fn Response::json(body: str) -> web.Response { web.json(body) }

// ponytail: 経路の関数をそのまま返す。Workers の fetch に渡す（web.worker）のは、インタプリタと C でも動く形が決まってから
fn hanabi(app: (web.Request) -> web.Response) -> (web.Request) -> web.Response { app }

// 1970 年からのミリ秒
fn Clock::now() -> int { now_ms() }

// ---- JSON ----

trait Json<A> {
    fn to_json(x: A) -> str
}

impl Json<int> {
    fn to_json(x: int) -> str { to_str(x) }
}

impl Json<bool> {
    fn to_json(x: bool) -> str { to_str(x) }
}

impl Json<str> {
    fn to_json(x: str) -> str { json_str(x) }
}

fn json_str(s: str) -> str {
    let mut out = '"'
    for c in chars(s) {
        out = out + if c == '"' { '\\"' }
        else if c == '\\' { '\\\\' }
        else if c == '\n' { '\\n' }
        else if c == '\r' { '\\r' }
        else if c == '\t' { '\\t' }
        else if code(c) < 32 { '\\u00' + hex(code(c) / 16) + hex(code(c) % 16) }
        else { c }
    }
    out + '"'
}

fn hex(d: int) -> str { slice('0123456789abcdef', d, d + 1) }

// ---- Durable Object ----

// wrangler.toml の binding の名前で指す名前空間
struct DurableObjectNamespace { binding: str }

// 名前空間の中の1つの Durable Object
struct DurableObject { binding: str, id: str, stamps: Ref<Vec<int>> }

fn durable_object(binding: str) -> DurableObjectNamespace { DurableObjectNamespace { binding: binding } }

// ponytail: Durable Object の状態を実行中のメモリに置くので、要求ごとに消える。Workers の binding（env.ACCOUNT）につなぐ時に置き換える
fn DurableObjectNamespace::get(ns: DurableObjectNamespace, id: Param) -> DurableObject {
    let stamps: Vec<int> = []
    DurableObject { binding: ns.binding, id: id.value, stamps: new_ref(stamps) }
}

// 時刻（ミリ秒）を記録する
fn DurableObject::stamp(o: DurableObject, t: int) { write(o.stamps, push(read(o.stamps), t)) }
