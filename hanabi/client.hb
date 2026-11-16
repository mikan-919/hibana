use std -> *
use './endpoint.hb'
use './types.hb' -> { Status, RequestId, method_name }
use './view.hb' -> { Html }
use './json.hb' -> { Codec }
use './dom.hb' -> { Dom, paint }

// 同期実行とテストでは応答を再開値として渡す。ブラウザーでは同じ要求を Command として保持する。
effect Transport { fn exchange(request: endpoint.Request) -> endpoint.Reply }
enum Command<M> { Idle, Fetch(endpoint.Request, (endpoint.Reply) -> M) }
fn call<P, I, O, E, M>(api: endpoint.Endpoint<P, I, O, E>, params: P, input: I, accept: (Result<O, E>) -> M) -> Command<M> {
    Fetch(endpoint.request(api, params, input), fn(reply) { accept(endpoint.response(api, reply)) })
}
fn perform<M>(command: Command<M>) -> Option<M> ! Transport {
    match command { Idle -> None Fetch(request, accept) -> Some(accept(exchange(request))) }
}
struct Change<S, M> { state: S, command: Command<M> }
fn stay<S, M>(state: S) -> Change<S, M> { Change { state: state, command: Idle } }
struct Program<S, M> {
    initial: S, update: (S, M) -> Change<S, M> ! Dom,
    view: (S) -> Html, event: (Json) -> Result<M, String>, root: String
}
struct Pending<M> { id: RequestId, accept: (endpoint.Reply) -> M }
struct Session<S, M> { program: Program<S, M>, state: S, next: RequestId, pending: Vec<Pending<M>> }
effect Schedule { fn enqueue(id: RequestId, request: endpoint.Request) }
fn create<S, M>(program: Program<S, M>) -> Ref<Session<S, M>> ! Dom {
    paint(program.root, (program.view)(program.initial))
    new_ref(Session { program: program, state: program.initial, next: 1, pending: [] })
}
fn dispatch<S, M>(session: Ref<Session<S, M>>, message: M) ! Dom, Schedule {
    let current = read(session)
    let change = (current.program.update)(current.state, message)
    let id = current.next
    let mut pending = current.pending
    match change.command { Idle -> {} Fetch(request, accept) -> { pending = push(pending, Pending { id: id, accept: accept }) } }
    write(session, Session { program: current.program, state: change.state, next: id + 1, pending: pending })
    paint(current.program.root, (current.program.view)(change.state))
    match change.command { Idle -> {} Fetch(request, _) -> enqueue(id, request) }
}
fn event<S, M>(session: Ref<Session<S, M>>, payload: String) ! Dom, Schedule {
    let program = read(session).program
    let json = match parse_json(payload) { Some(value) -> value None -> panic('イベントの JSON が不正') }
    let message = match (program.event)(json) { Ok(message) -> message Err(e) -> panic(e) }
    dispatch(session, message)
}
fn resolve<S, M>(session: Ref<Session<S, M>>, id: RequestId, status: Status, body: String) ! Dom, Schedule {
    let current = read(session)
    let mut remaining: Vec<Pending<M>> = []
    let mut message: Option<M> = None
    for pending in current.pending {
        if pending.id == id { message = Some((pending.accept)(endpoint.Reply { status: status, body: body })) }
        else { remaining = push(remaining, pending) }
    }
    write(session, Session { program: current.program, state: current.state, next: current.next, pending: remaining })
    match message { Some(message) -> dispatch(session, message) None -> {} }
}
fn browser<A>(action: () -> A ! Dom, Schedule) -> A {
    dom.browser(fn() -> A ! Dom {
        handle action() {
            enqueue(id, request) -> { dom.schedule(id, method_name(request.method), request.url, request.body); resume() }
        }
    })
}
