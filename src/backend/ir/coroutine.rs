//! async 関数をコルーチン（途中で止めて、後で続きから動かせる関数）にする変換。
//! JavaScript の async 関数と同じく、呼ぶと最初の await まで動いて Promise を返し、残りは仕事の並び（runtime.hb の __run_jobs）から動く。
//!
//! 関数 f は入口の f と、続きを動かす f$step に分ける。局所変数は区画（runtime.hb の __co_new）に置く。
//! f$step は、局所変数 0 に区画を受け取り、区画から局所変数を戻して、状態の番号の所から動く。
//! 本体は状態ごとの文の並びにし、`while true { if 状態 == 0 { ... } else if 状態 == 1 { ... } }` で振り分ける。
//! 状態を移るのは「状態の番号を置いて continue」。await では局所変数を区画に写し、再開する状態の番号を置いて戻る。
//!
//! 変換の前に、await とループを文の位置に出す（正規化）。await より前に評価する引数は局所変数に移し、
//! await を含む if・ブロック・Catch は文にする。ループはすべて状態に分けるので、状態の外に while は残らない。
//! そのため break・continue と、状態に分けた Catch への Escape は、どこに書いてあっても状態の移動にできる
//!
//! ponytail: 状態の振り分けは if の連なりなので、状態の数に比例する。状態が多い関数で遅ければ、WASM の br_table と Cranelift の jump table で飛ぶ
use super::*;

// 正規化した文。await・ループ・await を含む if と Catch だけが文の位置に来る
enum N {
    Plain(Stmt),
    // 局所変数に、値（局所変数か定数）を待った結果を置く
    Await(u32, Expr),
    Loop(Expr, Vec<N>),
    If(Expr, Vec<N>, Vec<N>),
    // 局所変数に Catch の値を置く。本体の Escape はこの Catch を抜ける（値は unit）
    Catch(u32, Vec<N>, Expr),
}

// await かループを含むか
fn complex(e: &Expr) -> bool {
    match e {
        Expr::Call(n, args) => n == "#await" || args.iter().any(complex),
        Expr::Prim(_, args) => args.iter().any(complex),
        Expr::CallIndirect(f, args) => complex(f) || args.iter().any(complex),
        Expr::If(c, t, e) => complex(c) || complex_block(t) || complex_block(e),
        Expr::Block(b) | Expr::Catch(b) => complex_block(b),
        Expr::Const(..) | Expr::Str(_) | Expr::Local(_) | Expr::Func(_) => false,
    }
}

fn complex_block(b: &Block) -> bool {
    complex(&b.value) || b.stmts.iter().any(complex_stmt)
}

fn complex_stmt(s: &Stmt) -> bool {
    match s {
        Stmt::Set(_, e) | Stmt::Eval(e) | Stmt::Return(e) => complex(e),
        Stmt::While(..) => true,
        Stmt::Break | Stmt::Continue | Stmt::Escape => false,
    }
}

// 局所変数の番号を 1 つずらす（0 番は区画）
fn shift(e: Expr) -> Expr {
    match e {
        Expr::Local(i) => Expr::Local(i + 1),
        Expr::Prim(p, args) => Expr::Prim(p, args.into_iter().map(shift).collect()),
        Expr::Call(n, args) => Expr::Call(n, args.into_iter().map(shift).collect()),
        Expr::CallIndirect(f, args) => Expr::CallIndirect(Box::new(shift(*f)), args.into_iter().map(shift).collect()),
        Expr::If(c, t, e) => Expr::If(Box::new(shift(*c)), Box::new(shift_block(*t)), Box::new(shift_block(*e))),
        Expr::Block(b) => Expr::Block(Box::new(shift_block(*b))),
        Expr::Catch(b) => Expr::Catch(Box::new(shift_block(*b))),
        e => e,
    }
}

fn shift_block(b: Block) -> Block {
    Block { stmts: shift_stmts(b.stmts), value: shift(b.value) }
}

fn shift_stmts(ss: Vec<Stmt>) -> Vec<Stmt> {
    ss.into_iter()
        .map(|s| match s {
            Stmt::Set(i, e) => Stmt::Set(i + 1, shift(e)),
            Stmt::Eval(e) => Stmt::Eval(shift(e)),
            Stmt::Return(e) => Stmt::Return(shift(e)),
            Stmt::While(c, body) => Stmt::While(shift(c), shift_stmts(body)),
            s => s,
        })
        .collect()
}

// 区画の k 番目の値の番地。0 Promise、1 状態の番号、2 再開の状態、3 再開の値、4 から局所変数 1 番以降
fn cap(frame: u32, k: u32) -> Expr {
    add(prim(Prim::Payload, vec![Expr::Local(frame)]), int(16 + 16 * k as i64))
}

struct Co {
    next: u32,
    state: u32,
    states: Vec<Vec<Stmt>>,
    // 状態に分けたループの (先頭, 後ろ) と Catch の (後ろ, 値を置く局所変数)
    loops: Vec<(usize, usize)>,
    catches: Vec<(usize, u32)>,
}

impl Co {
    fn local(&mut self) -> u32 {
        self.next += 1;
        self.next - 1
    }

    fn spill(&mut self, v: Expr, out: &mut Vec<N>) -> Expr {
        if matches!(v, Expr::Const(..) | Expr::Str(_) | Expr::Func(_)) {
            return v;
        }
        let t = self.local();
        out.push(N::Plain(Stmt::Set(t, v)));
        Expr::Local(t)
    }

    fn norm_stmts(&mut self, ss: Vec<Stmt>) -> Vec<N> {
        let mut out = Vec::new();
        for s in ss {
            self.norm_stmt(s, &mut out);
        }
        out
    }

    fn norm_stmt(&mut self, s: Stmt, out: &mut Vec<N>) {
        match s {
            Stmt::Set(i, e) if complex(&e) => {
                let v = self.norm_expr(e, out);
                out.push(N::Plain(Stmt::Set(i, v)));
            }
            Stmt::Eval(e) if complex(&e) => {
                let v = self.norm_expr(e, out);
                if !matches!(v, Expr::Const(..) | Expr::Str(_) | Expr::Local(_) | Expr::Func(_)) {
                    out.push(N::Plain(Stmt::Eval(v)));
                }
            }
            Stmt::Return(e) if complex(&e) => {
                let v = self.norm_expr(e, out);
                out.push(N::Plain(Stmt::Return(v)));
            }
            Stmt::While(c, body) if complex(&c) => {
                // while c { 本体 } は while true { if !c { break } 本体 }
                let mut inner = Vec::new();
                let c = self.norm_expr(c, &mut inner);
                let exit = Block { stmts: vec![Stmt::Break], value: unit() };
                inner.push(N::Plain(Stmt::Eval(Expr::If(Box::new(prim(Prim::Not, vec![c])), Box::new(exit), leaf(unit())))));
                inner.extend(self.norm_stmts(body));
                out.push(N::Loop(Expr::Const(BOOL, 1), inner));
            }
            Stmt::While(c, body) => {
                let body = self.norm_stmts(body);
                out.push(N::Loop(c, body));
            }
            s => out.push(N::Plain(s)),
        }
    }

    fn norm_args(&mut self, args: Vec<Expr>, out: &mut Vec<N>) -> Vec<Expr> {
        let last = args.iter().rposition(complex);
        args.into_iter()
            .enumerate()
            .map(|(i, a)| match last {
                Some(l) if i < l => {
                    let v = self.norm_expr(a, out);
                    self.spill(v, out)
                }
                Some(l) if i == l => self.norm_expr(a, out),
                _ => a,
            })
            .collect()
    }

    fn norm_block(&mut self, b: Block, out: &mut Vec<N>) -> Expr {
        for s in b.stmts {
            self.norm_stmt(s, out);
        }
        self.norm_expr(b.value, out)
    }

    fn norm_expr(&mut self, e: Expr, out: &mut Vec<N>) -> Expr {
        if !complex(&e) {
            return e;
        }
        match e {
            Expr::Call(n, mut args) if n == "#await" => {
                let x = self.norm_expr(args.remove(0), out);
                let x = self.spill(x, out);
                let t = self.local();
                out.push(N::Await(t, x));
                Expr::Local(t)
            }
            Expr::Call(n, args) => Expr::Call(n, self.norm_args(args, out)),
            Expr::Prim(p, args) => Expr::Prim(p, self.norm_args(args, out)),
            Expr::CallIndirect(f, args) => {
                let mut all = vec![*f];
                all.extend(args);
                let mut all = self.norm_args(all, out);
                let f = all.remove(0);
                Expr::CallIndirect(Box::new(f), all)
            }
            Expr::Block(b) => self.norm_block(*b, out),
            Expr::If(c, t, e) => {
                let c = self.norm_expr(*c, out);
                let r = self.local();
                let mut ts = Vec::new();
                let v = self.norm_block(*t, &mut ts);
                ts.push(N::Plain(Stmt::Set(r, v)));
                let mut es = Vec::new();
                let v = self.norm_block(*e, &mut es);
                es.push(N::Plain(Stmt::Set(r, v)));
                out.push(N::If(c, ts, es));
                Expr::Local(r)
            }
            Expr::Catch(b) => {
                let r = self.local();
                let mut inner = Vec::new();
                let v = self.norm_block(*b, &mut inner);
                out.push(N::Catch(r, inner, v));
                Expr::Local(r)
            }
            e => e,
        }
    }

    fn new_state(&mut self) -> usize {
        self.states.push(Vec::new());
        self.states.len() - 1
    }

    fn goto(&self, k: usize) -> Vec<Stmt> {
        vec![Stmt::Set(self.state, int(k as i64)), Stmt::Continue]
    }

    fn flatten(&mut self, ns: Vec<N>, mut cur: usize) -> usize {
        for n in ns {
            match n {
                N::Plain(s) => {
                    let s = self.rewrite_stmt(s, 0);
                    self.states[cur].extend(s);
                }
                N::Await(t, x) => {
                    let k = self.new_state();
                    let mut code = setv(cap(0, 1), int(k as i64));
                    for j in 1..self.state {
                        code.extend(setv(cap(0, 3 + j), Expr::Local(j)));
                    }
                    code.push(Stmt::Eval(call("__await", vec![Expr::Local(0), x])));
                    code.push(Stmt::Return(unit()));
                    self.states[cur].extend(code);
                    cur = k;
                    self.states[k].push(Stmt::Set(t, call("__co_resume", vec![Expr::Local(0)])));
                }
                N::Loop(c, body) => {
                    let (head, after) = (self.new_state(), self.new_state());
                    let jump = self.goto(head);
                    self.states[cur].extend(jump);
                    let c = self.rewrite(c, 0);
                    let exit = Block { stmts: self.goto(after), value: unit() };
                    self.states[head].push(Stmt::Eval(Expr::If(Box::new(prim(Prim::Not, vec![c])), Box::new(exit), leaf(unit()))));
                    self.loops.push((head, after));
                    let end = self.flatten(body, head);
                    self.loops.pop();
                    let jump = self.goto(head);
                    self.states[end].extend(jump);
                    cur = after;
                }
                N::If(c, t, e) => {
                    let (ts, es, after) = (self.new_state(), self.new_state(), self.new_state());
                    let c = self.rewrite(c, 0);
                    let (yes, no) = (Block { stmts: self.goto(ts), value: unit() }, Block { stmts: self.goto(es), value: unit() });
                    self.states[cur].push(Stmt::Eval(Expr::If(Box::new(c), Box::new(yes), Box::new(no))));
                    for (branch, start) in [(t, ts), (e, es)] {
                        let end = self.flatten(branch, start);
                        let jump = self.goto(after);
                        self.states[end].extend(jump);
                    }
                    cur = after;
                }
                N::Catch(r, body, v) => {
                    let after = self.new_state();
                    self.catches.push((after, r));
                    let end = self.flatten(body, cur);
                    // 値の式の Escape も、この Catch を抜ける
                    let v = self.rewrite(v, 0);
                    self.catches.pop();
                    self.states[end].push(Stmt::Set(r, v));
                    let jump = self.goto(after);
                    self.states[end].extend(jump);
                    cur = after;
                }
            }
        }
        cur
    }

    // 状態に分けた後の文。return は Promise を決めて戻る。break・continue と、状態に分けた Catch への Escape は状態の移動にする。
    // depth は、この文の中で囲んでいる（状態に分けていない）Catch の数
    fn rewrite_stmt(&self, s: Stmt, depth: usize) -> Vec<Stmt> {
        match s {
            Stmt::Return(e) => vec![
                Stmt::Eval(call("__co_return", vec![Expr::Local(0), self.rewrite(e, depth)])),
                Stmt::Return(unit()),
            ],
            Stmt::Break => self.goto(self.loops.last().expect("ループの中だけ").1),
            Stmt::Continue => self.goto(self.loops.last().expect("ループの中だけ").0),
            Stmt::Escape if depth == 0 => {
                let (after, r) = *self.catches.last().expect("Catch の中だけ");
                let mut out = vec![Stmt::Set(r, unit())];
                out.extend(self.goto(after));
                out
            }
            Stmt::Set(i, e) => vec![Stmt::Set(i, self.rewrite(e, depth))],
            Stmt::Eval(e) => vec![Stmt::Eval(self.rewrite(e, depth))],
            Stmt::While(..) => unreachable!("ループは正規化で状態に分けている"),
            s => vec![s],
        }
    }

    fn rewrite_block(&self, b: Block, depth: usize) -> Block {
        Block {
            stmts: b.stmts.into_iter().flat_map(|s| self.rewrite_stmt(s, depth)).collect(),
            value: self.rewrite(b.value, depth),
        }
    }

    fn rewrite(&self, e: Expr, depth: usize) -> Expr {
        match e {
            Expr::Prim(p, args) => Expr::Prim(p, args.into_iter().map(|a| self.rewrite(a, depth)).collect()),
            Expr::Call(n, args) => Expr::Call(n, args.into_iter().map(|a| self.rewrite(a, depth)).collect()),
            Expr::CallIndirect(f, args) => {
                Expr::CallIndirect(Box::new(self.rewrite(*f, depth)), args.into_iter().map(|a| self.rewrite(a, depth)).collect())
            }
            Expr::If(c, t, e) => Expr::If(Box::new(self.rewrite(*c, depth)), Box::new(self.rewrite_block(*t, depth)), Box::new(self.rewrite_block(*e, depth))),
            Expr::Block(b) => Expr::Block(Box::new(self.rewrite_block(*b, depth))),
            Expr::Catch(b) => Expr::Catch(Box::new(self.rewrite_block(*b, depth + 1))),
            e => e,
        }
    }
}

impl Ctx<'_> {
    // async 関数 f をコルーチンにする。続きの関数 f$step を made に足し、入口の関数（f の名前）を返す
    pub(super) fn coroutine(&mut self, f: Function) -> Function {
        let step_name = format!("{}$step", f.name);
        let mut co = Co { next: f.locals + 1, state: 0, states: vec![Vec::new()], loops: Vec::new(), catches: Vec::new() };
        let body = co.norm_stmts(shift_stmts(f.body));
        co.state = co.local();
        let end = co.flatten(body, 0);
        // 本体は return で終わる。念のため、終わりに来たら unit で戻る
        let finish = co.rewrite_stmt(Stmt::Return(unit()), 0);
        co.states[end].extend(finish);
        let mut dispatch = unit();
        for (k, code) in std::mem::take(&mut co.states).into_iter().enumerate().rev() {
            let here = prim(Prim::Eq, vec![Expr::Local(co.state), int(k as i64)]);
            dispatch = Expr::If(Box::new(here), Box::new(Block { stmts: code, value: unit() }), leaf(dispatch));
        }
        let mut body = vec![Stmt::Set(co.state, prim(Prim::Payload, vec![getv(cap(0, 1))]))];
        for j in 1..co.state {
            body.push(Stmt::Set(j, getv(cap(0, 3 + j))));
        }
        body.push(Stmt::While(Expr::Const(BOOL, 1), vec![Stmt::Eval(dispatch)]));
        body.push(Stmt::Return(unit()));
        let mut step = Function {
            name: step_name.clone(),
            params: 1,
            param_types: vec![],
            result_type: ValueType::Other,
            export_error: None,
            finish_at_boundary: false,
            locals: co.state + 1,
            body,
        };
        self.shadow(&mut step);
        self.made.push(step);
        let frame = f.params;
        let mut body = vec![Stmt::Set(frame, call("__co_new", vec![Expr::Func(step_name.clone()), int(co.state as i64 - 1)]))];
        for i in 0..f.params {
            body.extend(setv(cap(frame, 4 + i), Expr::Local(i)));
        }
        body.push(Stmt::Eval(call("__co_step", vec![Expr::Local(frame)])));
        body.push(Stmt::Return(call("__co_promise", vec![Expr::Local(frame)])));
        let mut entry = Function { locals: f.params + 1, body, ..f };
        self.shadow(&mut entry);
        entry
    }
}
