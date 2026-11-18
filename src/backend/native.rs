// 構造化 IR を Cranelift で機械語にし、libc を使わない静的な ELF の実行ファイルを作る。
// リンカーは使わない。機械語と文字列を RX、可変状態を RW の区画に並べ、再配置を自分で埋める。
// IR の値（型の番号と中身）は 2 つの i64 で表し、関数は 2 つの i64 を返す。
// ponytail: x86-64 の Linux だけ。aarch64 は syscall の機械語（SYSCALL・START）を足せば動く。macOS・Windows は OS のライブラリを通す別の出力が要る
use crate::error::R;
use crate::ir::{self, Block, Expr, Node, Prim, Program, Stmt};
use cranelift_codegen::binemit::Reloc;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::condcodes::{IntCC, FloatCC};
use cranelift_codegen::ir::types::{I64, F64};
use cranelift_codegen::ir::{
    AbiParam, ExtFuncData, ExternalName, FuncRef, Function, GlobalValueData, InstBuilder,
    MemFlagsData, Signature, StackSlotData, StackSlotKind, TrapCode, UserExternalName,
    UserFuncName, Value,
};
use cranelift_codegen::isa::{CallConv, OwnedTargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::{Context, RelocTarget};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use std::collections::HashMap;

// 読み込む位置。Linux の静的な実行ファイルの慣例に合わせる
const BASE: u64 = 0x400000;
// ELF ヘッダー（64 バイト）とプログラムヘッダー 2 つ（各 56 バイト）の後
const CODE_START: usize = 176;
const PAGE_SIZE: usize = 0x1000;
// 名前の区分。UserExternalName の namespace に入れる
const FUNC: u32 = 0;
const DATA: u32 = 1;

// syscall(番号, a, b, c, d, e) -> 結果。System V の引数のレジスタを Linux の syscall のレジスタへ移す
const SYSCALL: &[u8] = &[
    0x48, 0x89, 0xf8, // mov rax, rdi
    0x48, 0x89, 0xf7, // mov rdi, rsi
    0x48, 0x89, 0xd6, // mov rsi, rdx
    0x48, 0x89, 0xca, // mov rdx, rcx
    0x4d, 0x89, 0xc2, // mov r10, r8
    0x4d, 0x89, 0xc8, // mov r8, r9
    0x45, 0x31, 0xc9, // xor r9d, r9d
    0x0f, 0x05, // syscall
    0xc3, // ret
];
// 入口。スタックを 16 バイトにそろえて main を呼び、終了コード 0 で抜ける
const START: &[u8] = &[
    0x31, 0xed, // xor ebp, ebp
    0x48, 0x83, 0xe4, 0xf0, // and rsp, -16
    0xe8, 0, 0, 0, 0, // call main（再配置で埋める）
    0x31, 0xff, // xor edi, edi
    0xb8, 0x3c, 0, 0, 0, // mov eax, 60（exit）
    0x0f, 0x05, // syscall
];
const START_CALL: u32 = 7;

const SYS_WRITE: i64 = 1;
const SYS_MMAP: i64 = 9;
const SYS_EXIT: i64 = 60;
const SYS_CLOCK_GETTIME: i64 = 228;

struct Code {
    bytes: Vec<u8>,
    // (位置, 種類, 区分, 番号, 加数)
    relocs: Vec<(u32, Reloc, u32, u32, i64)>,
}

pub fn emit(program: &Program) -> R<Vec<u8>> {
    if !cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        return Err("ネイティブ出力は x86-64 の Linux だけに対応".into());
    }
    if let Some(f) = program.imports.first() {
        return Err(format!("ネイティブ出力は外部関数 {} に未対応", f.name).into());
    }
    let main = program
        .functions
        .iter()
        .position(|f| f.name == "main")
        .ok_or("ネイティブ出力には main が必要")?;
    let entry = program.functions[main].finish_at_boundary.then(|| ir::Function {
        name: "main$entry".into(),
        params: 0,
        param_types: vec![],
        result_type: ir::ValueType::Unit,
        finish_at_boundary: false,
        locals: 0,
        body: vec![
            Stmt::Eval(Expr::Call("__begin_entry".into(), vec![])),
            Stmt::Return(Expr::Call(
                "__finish_entry".into(),
                vec![Expr::Call("main".into(), vec![])],
            )),
        ],
    });
    let main = if entry.is_some() { program.functions.len() } else { main } as u32;
    let functions = || program.functions.iter().chain(entry.iter());
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    let isa = cranelift_native::builder()
        .map_err(|e| format!("この CPU には未対応: {e}"))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| format!("Cranelift の設定に失敗した: {e}"))?;
    let n = functions().count() as u32;
    let mut g = Gen {
        isa,
        ctx: Context::new(),
        fctx: FunctionBuilderContext::new(),
        indices: functions()
            .enumerate()
            .map(|(i, f)| (f.name.clone(), i as u32))
            .collect(),
        literals: HashMap::new(),
        datas: Vec::new(),
        syscall: n,
    };
    // ランタイムの状態（runtime.hb の __static）。最初の確保で mmap する
    g.datas.push(vec![0; ir::STATIC_BYTES]);
    for s in ["実行時エラー: ", "\n"] {
        g.literal(s);
    }
    for f in functions() {
        f.each(&mut |n| {
            if let Node::Expr(Expr::Str(s)) = n {
                g.literal(s);
            }
        });
    }
    let mut codes = Vec::new();
    for f in functions() {
        let func = g.function(f);
        codes.push(g.compile(func)?);
    }
    codes.push(Code {
        bytes: SYSCALL.to_vec(),
        relocs: vec![],
    });
    codes.push(Code {
        bytes: START.to_vec(),
        relocs: vec![(START_CALL, Reloc::X86CallPCRel4, FUNC, main, -4)],
    });
    Ok(link(&codes, &g.datas))
}

struct Gen {
    isa: OwnedTargetIsa,
    ctx: Context,
    fctx: FunctionBuilderContext,
    indices: HashMap<String, u32>,
    // 文字列は [長さ: 8 バイト][UTF-8 のバイト列] の形で置く
    literals: HashMap<String, u32>,
    datas: Vec<Vec<u8>>,
    syscall: u32,
}

fn signature(params: usize, results: usize) -> Signature {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.params = vec![AbiParam::new(I64); params];
    sig.returns = vec![AbiParam::new(I64); results];
    sig
}

impl Gen {
    fn literal(&mut self, s: &str) -> u32 {
        if let Some(i) = self.literals.get(s) {
            return *i;
        }
        let mut bytes = (s.len() as u64).to_le_bytes().to_vec();
        bytes.extend(s.as_bytes());
        self.datas.push(bytes);
        self.literals
            .insert(s.to_string(), self.datas.len() as u32 - 1);
        self.datas.len() as u32 - 1
    }

    fn compile(&mut self, func: Function) -> R<Code> {
        self.ctx.func = func;
        let (bytes, relocs) = {
            let code = self
                .ctx
                .compile(&*self.isa, &mut ControlPlane::default())
                .map_err(|e| format!("ネイティブのコード生成に失敗した: {:?}", e.inner))?;
            (code.buffer.data().to_vec(), code.buffer.relocs().to_vec())
        };
        let names = self.ctx.func.params.user_named_funcs();
        let relocs = relocs
            .iter()
            .map(|r| {
                let RelocTarget::ExternalName(ExternalName::User(name)) = &r.target else {
                    return Err(format!("未対応の再配置先: {:?}", r.target).into());
                };
                let name = &names[*name];
                Ok((r.offset, r.kind, name.namespace, name.index, r.addend))
            })
            .collect::<R<Vec<_>>>()?;
        self.ctx.clear();
        Ok(Code { bytes, relocs })
    }

    fn function(&mut self, f: &ir::Function) -> Function {
        let mut func = Function::with_name_signature(
            UserFuncName::default(),
            signature(2 * f.params as usize, 2),
        );
        let config = self.isa.frontend_config();
        let mut b = FunctionBuilder::new(&mut func, &mut self.fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let vars: Vec<Variable> = (0..2 * f.locals).map(|_| b.declare_var(I64)).collect();
        let zero = b.ins().iconst(I64, 0);
        for (i, v) in vars.iter().enumerate() {
            let init = b.block_params(entry).get(i).copied().unwrap_or(zero);
            b.def_var(*v, init);
        }
        let mut e = Body {
            b,
            vars,
            funcs: HashMap::new(),
            indices: &self.indices,
            literals: &self.literals,
            syscall: self.syscall,
            catches: Vec::new(),
            loops: Vec::new(),
        };
        e.stmts(&f.body);
        let unit = e.b.ins().iconst(I64, ir::UNIT);
        let zero = e.b.ins().iconst(I64, 0);
        e.b.ins().return_(&[unit, zero]);
        e.b.seal_all_blocks();
        e.b.finalize(config);
        func
    }
}

fn import(b: &mut FunctionBuilder, index: u32, params: usize, results: usize) -> FuncRef {
    let signature = b.import_signature(signature(params, results));
    let name = b
        .func
        .declare_imported_user_function(UserExternalName::new(FUNC, index));
    b.import_function(ExtFuncData {
        name: ExternalName::user(name),
        signature,
        colocated: true,
        patchable: false,
    })
}

struct Body<'a, 'f> {
    b: FunctionBuilder<'f>,
    vars: Vec<Variable>,
    funcs: HashMap<(u32, usize), FuncRef>,
    indices: &'a HashMap<String, u32>,
    literals: &'a HashMap<String, u32>,
    syscall: u32,
    // 入れ子の Catch の後の区画。Escape はいちばん内側へ飛ぶ
    catches: Vec<cranelift_codegen::ir::Block>,
    loops: Vec<(cranelift_codegen::ir::Block, cranelift_codegen::ir::Block)>,
}

impl Body<'_, '_> {
    fn func(&mut self, index: u32, params: usize, results: usize) -> FuncRef {
        // 同じ関数でも、番地だけを取る所（Expr::Func）と呼ぶ所で引数の数が違うので、数ごとに分ける
        if let Some(f) = self.funcs.get(&(index, params)) {
            return *f;
        }
        let f = import(&mut self.b, index, params, results);
        self.funcs.insert((index, params), f);
        f
    }

    fn call_ir(&mut self, name: &str, args: &[Value]) -> (Value, Value) {
        let index = self.indices[name];
        let f = self.func(index, args.len(), 2);
        let call = self.b.ins().call(f, args);
        let r = self.b.inst_results(call);
        (r[0], r[1])
    }

    fn data(&mut self, index: u32) -> Value {
        let name = self
            .b
            .func
            .declare_imported_user_function(UserExternalName::new(DATA, index));
        let gv = self.b.create_global_value(GlobalValueData::Symbol {
            name: ExternalName::user(name),
            offset: 0.into(),
            colocated: true,
            tls: false,
        });
        self.b.ins().symbol_value(I64, gv)
    }

    fn sys(&mut self, nr: i64, args: &[Value]) -> Value {
        let mut all = vec![self.b.ins().iconst(I64, nr)];
        all.extend(args);
        while all.len() < 6 {
            all.push(self.b.ins().iconst(I64, 0));
        }
        let f = self.func(self.syscall, 6, 1);
        let call = self.b.ins().call(f, &all);
        self.b.inst_results(call)[0]
    }

    fn int(&mut self, n: i64) -> Value {
        self.b.ins().iconst(I64, n)
    }

    fn flags() -> MemFlagsData {
        MemFlagsData::trusted()
    }

    // 条件が真なら、ランタイムの関数（戻らない）を呼ぶ
    fn fail_if(&mut self, cond: Value, name: &str) {
        let (fail, ok) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(cond, fail, &[], ok, &[]);
        self.b.switch_to_block(fail);
        self.b.set_cold_block(fail);
        self.call_ir(name, &[]);
        self.b.ins().trap(TrapCode::unwrap_user(1));
        self.b.switch_to_block(ok);
    }

    // 分岐や return の後の到達しない位置に、続きを書くための区画を作る
    fn unreachable(&mut self) {
        let next = self.b.create_block();
        self.b.switch_to_block(next);
    }

    fn stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::Set(i, e) => {
                    let (t, p) = self.expr(e);
                    self.b.def_var(self.vars[2 * *i as usize], t);
                    self.b.def_var(self.vars[2 * *i as usize + 1], p);
                }
                Stmt::Eval(e) => {
                    self.expr(e);
                }
                Stmt::While(c, body) => {
                    let (head, inner, exit) = (
                        self.b.create_block(),
                        self.b.create_block(),
                        self.b.create_block(),
                    );
                    self.b.ins().jump(head, &[]);
                    self.b.switch_to_block(head);
                    let c = self.expr(c).1;
                    self.b.ins().brif(c, inner, &[], exit, &[]);
                    self.b.switch_to_block(inner);
                    self.loops.push((head, exit));
                    self.stmts(body);
                    self.loops.pop();
                    self.b.ins().jump(head, &[]);
                    self.b.switch_to_block(exit);
                }
                Stmt::Break | Stmt::Continue => {
                    let (head, exit) = *self.loops.last().expect("ループの本体の中だけ");
                    let target = if matches!(s, Stmt::Break) { exit } else { head };
                    self.b.ins().jump(target, &[]);
                    self.unreachable();
                }
                Stmt::Return(e) => {
                    let (t, p) = self.expr(e);
                    self.b.ins().return_(&[t, p]);
                    self.unreachable();
                }
                Stmt::Escape => {
                    let target = *self.catches.last().expect("Escape は Catch の中だけ");
                    let (t, p) = (self.int(ir::UNIT), self.int(0));
                    self.b.ins().jump(target, &[t.into(), p.into()]);
                    self.unreachable();
                }
            }
        }
    }

    fn block(&mut self, b: &Block) -> (Value, Value) {
        self.stmts(&b.stmts);
        self.expr(&b.value)
    }

    fn expr(&mut self, e: &Expr) -> (Value, Value) {
        match e {
            Expr::Const(t, n) => (self.int(*t), self.int(*n)),
            Expr::Str(s) => {
                let p = self.data(self.literals[s]);
                (self.int(ir::STR), p)
            }
            Expr::Local(i) => (
                self.b.use_var(self.vars[2 * *i as usize]),
                self.b.use_var(self.vars[2 * *i as usize + 1]),
            ),
            Expr::Func(name) => {
                let index = self.indices[name];
                // 番地だけが要る。呼ぶ時の型は call_indirect で決まる
                let f = self.func(index, 0, 2);
                let p = self.b.ins().func_addr(I64, f);
                (self.int(ir::INT), p)
            }
            Expr::Call(name, args) => {
                let mut vs = Vec::new();
                for a in args {
                    let (t, p) = self.expr(a);
                    vs.extend([t, p]);
                }
                self.call_ir(name, &vs)
            }
            Expr::CallIndirect(g, args) => {
                let (gt, gp) = self.expr(g);
                let mut vs = vec![gt, gp];
                for a in args {
                    let (t, p) = self.expr(a);
                    vs.extend([t, p]);
                }
                let code = self.b.ins().load(I64, Self::flags(), gp, 0);
                let sig = self.b.import_signature(signature(vs.len(), 2));
                let call = self.b.ins().call_indirect(sig, code, &vs);
                let r = self.b.inst_results(call);
                (r[0], r[1])
            }
            Expr::If(c, t, f) => {
                let c = self.expr(c).1;
                let (then, other, merge) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                self.b.append_block_param(merge, I64);
                self.b.append_block_param(merge, I64);
                self.b.ins().brif(c, then, &[], other, &[]);
                self.b.switch_to_block(then);
                let (vt, vp) = self.block(t);
                self.b.ins().jump(merge, &[vt.into(), vp.into()]);
                self.b.switch_to_block(other);
                let (vt, vp) = self.block(f);
                self.b.ins().jump(merge, &[vt.into(), vp.into()]);
                self.b.switch_to_block(merge);
                let ps = self.b.block_params(merge);
                (ps[0], ps[1])
            }
            Expr::Block(b) => self.block(b),
            // 抜けた時は Escape が unit を持って、この塊の後へ飛ぶ
            Expr::Catch(b) => {
                let merge = self.b.create_block();
                self.b.append_block_param(merge, I64);
                self.b.append_block_param(merge, I64);
                self.catches.push(merge);
                let (vt, vp) = self.block(b);
                self.catches.pop();
                self.b.ins().jump(merge, &[vt.into(), vp.into()]);
                self.b.switch_to_block(merge);
                let ps = self.b.block_params(merge);
                (ps[0], ps[1])
            }
            Expr::Prim(p, args) => self.prim(*p, args),
        }
    }

    fn prim(&mut self, p: Prim, args: &[Expr]) -> (Value, Value) {
        use Prim::*;
        let vs: Vec<(Value, Value)> = args.iter().map(|a| self.expr(a)).collect();
        let x = |k: usize| vs[k].1;
        let payload = match p {
            Tag => vs[0].0,
            Payload => x(0),
            MakeV => return (x(0), x(1)),
            Add | Sub | Mul => {
                let (r, of) = match p {
                    Add => self.b.ins().sadd_overflow(x(0), x(1)),
                    Sub => self.b.ins().ssub_overflow(x(0), x(1)),
                    _ => self.b.ins().smul_overflow(x(0), x(1)),
                };
                self.fail_if(of, "__overflow");
                r
            }
            Div | Rem => {
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, x(1), 0);
                self.fail_if(zero, "__div_zero");
                let min = self.b.ins().icmp_imm_s(IntCC::Equal, x(0), i64::MIN);
                let minus_one = self.b.ins().icmp_imm_s(IntCC::Equal, x(1), -1);
                let of = self.b.ins().band(min, minus_one);
                self.fail_if(of, "__overflow");
                if p == Div {
                    self.b.ins().sdiv(x(0), x(1))
                } else {
                    self.b.ins().srem(x(0), x(1))
                }
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                let cc = match p {
                    Eq => IntCC::Equal,
                    Ne => IntCC::NotEqual,
                    Lt => IntCC::SignedLessThan,
                    Le => IntCC::SignedLessThanOrEqual,
                    Gt => IntCC::SignedGreaterThan,
                    _ => IntCC::SignedGreaterThanOrEqual,
                };
                let c = self.b.ins().icmp(cc, x(0), x(1));
                let v = self.b.ins().uextend(I64, c);
                return (self.int(ir::BOOL), v);
            }
            Not => {
                let c = self.b.ins().icmp_imm_s(IntCC::Equal, x(0), 0);
                let v = self.b.ins().uextend(I64, c);
                return (self.int(ir::BOOL), v);
            }
            And => self.b.ins().band(x(0), x(1)),
            Or => self.b.ins().bor(x(0), x(1)),
            Shl => self.b.ins().ishl(x(0), x(1)),
            Shr => self.b.ins().ushr(x(0), x(1)),
            Load => self.b.ins().load(I64, Self::flags(), x(0), 0),
            Load8 => self.b.ins().uload8(I64, Self::flags(), x(0), 0),
            Store | Store8 => {
                if p == Store {
                    self.b.ins().store(Self::flags(), x(1), x(0), 0);
                } else {
                    self.b.ins().istore8(Self::flags(), x(1), x(0), 0);
                }
                return (self.int(ir::UNIT), self.int(0));
            }
            // 読み書きできる無名のメモリを借りる。失敗（負の値）なら止める
            Pages => {
                let (zero, prot, map, fd) =
                    (self.int(0), self.int(3), self.int(0x22), self.int(-1));
                let r = self.sys(SYS_MMAP, &[zero, x(0), prot, map, fd]);
                let bad = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, r, 0);
                self.b.ins().trapnz(bad, TrapCode::unwrap_user(1));
                r
            }
            Static => self.data(0),
            Write => {
                self.sys(SYS_WRITE, &[x(0), x(1), x(2)]);
                return (self.int(ir::UNIT), self.int(0));
            }
            Fail => {
                let two = self.int(2);
                let prefix = self.data(self.literals["実行時エラー: "]);
                let prefix = self.b.ins().iadd_imm_s(prefix, 8);
                let prefix_len = self.int("実行時エラー: ".len() as i64);
                self.sys(SYS_WRITE, &[two, prefix, prefix_len]);
                let len = self.b.ins().load(I64, Self::flags(), x(0), 0);
                let bytes = self.b.ins().iadd_imm_s(x(0), 8);
                self.sys(SYS_WRITE, &[two, bytes, len]);
                let nl = self.data(self.literals["\n"]);
                let nl = self.b.ins().iadd_imm_s(nl, 8);
                let one = self.int(1);
                self.sys(SYS_WRITE, &[two, nl, one]);
                self.sys(SYS_EXIT, &[one]);
                self.b.ins().trap(TrapCode::unwrap_user(1));
                self.unreachable();
                return (self.int(ir::UNIT), self.int(0));
            }
            Now => {
                let slot = self.b.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    16,
                    3,
                ));
                let ts = self.b.ins().stack_addr(I64, slot, 0);
                let clock = self.int(0);
                self.sys(SYS_CLOCK_GETTIME, &[clock, ts]);
                let sec = self.b.ins().load(I64, Self::flags(), ts, 0);
                let nsec = self.b.ins().load(I64, Self::flags(), ts, 8);
                let ms = self.b.ins().imul_imm_s(sec, 1000);
                let million = self.int(1_000_000);
                let sub = self.b.ins().sdiv(nsec, million);
                self.b.ins().iadd(ms, sub)
            }
            Read => self.sys(0, &[x(0), x(1), x(2)]),
            FFromInt => {
                let f = self.b.ins().fcvt_from_sint(F64, x(0));
                self.b.ins().bitcast(I64, MemFlagsData::new(), f)
            }
            FToInt | FAdd | FSub | FMul | FDiv | FNeg | FFloor | FSqrt | FEq | FLt | FLe => {
                let a = self.b.ins().bitcast(F64, MemFlagsData::new(), x(0));
                if p == FToInt { self.b.ins().fcvt_to_sint_sat(I64, a) } else {
                    let b = if vs.len() > 1 { self.b.ins().bitcast(F64, MemFlagsData::new(), x(1)) } else { a };
                    if matches!(p, FEq | FLt | FLe) {
                        let cc = match p { FEq => FloatCC::Equal, FLt => FloatCC::LessThan, _ => FloatCC::LessThanOrEqual };
                        let r = self.b.ins().fcmp(cc, a, b);
                        let r = self.b.ins().uextend(I64, r);
                        return (self.int(ir::BOOL), r);
                    }
                    let r = match p {
                        FAdd => self.b.ins().fadd(a,b), FSub => self.b.ins().fsub(a,b), FMul => self.b.ins().fmul(a,b), FDiv => self.b.ins().fdiv(a,b),
                        FNeg => self.b.ins().fneg(a), FFloor => self.b.ins().floor(a), FSqrt => self.b.ins().sqrt(a), _ => unreachable!(),
                    };
                    self.b.ins().bitcast(I64, MemFlagsData::new(), r)
                }
            }
        };
        (self.int(ir::INT), payload)
    }
}

// 関数とデータを並べて再配置を埋め、ELF を書く
fn link(codes: &[Code], datas: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0u8; CODE_START];
    let mut funcs = Vec::new();
    for c in codes {
        while out.len() % 16 != 0 {
            out.push(0xcc);
        }
        funcs.push(out.len());
        out.extend(&c.bytes);
    }
    let mut places = vec![0; datas.len()];
    // 文字列定数は機械語と同じ RX の区画に置く。
    for (i, d) in datas.iter().enumerate().skip(1) {
        while out.len() % 8 != 0 {
            out.push(0);
        }
        places[i] = out.len();
        out.extend(d);
    }
    let rx_size = out.len();
    // __static だけを RW の区画に置き、RX とページを共有させない。
    let rw_start = rx_size.next_multiple_of(PAGE_SIZE);
    out.resize(rw_start, 0);
    places[0] = rw_start;
    out.extend(&datas[0]);
    let addr = |offset: usize| BASE + offset as u64;
    for (c, start) in codes.iter().zip(&funcs) {
        for &(offset, kind, namespace, index, addend) in &c.relocs {
            let target = if namespace == FUNC {
                funcs[index as usize]
            } else {
                places[index as usize]
            };
            let at = start + offset as usize;
            let value = addr(target) as i64 + addend;
            match kind {
                Reloc::Abs8 => out[at..at + 8].copy_from_slice(&value.to_le_bytes()),
                Reloc::X86PCRel4 | Reloc::X86CallPCRel4 | Reloc::X86CallPLTRel4 => {
                    let rel = i32::try_from(value - addr(at) as i64)
                        .expect("再配置の距離が 32 ビットを超える");
                    out[at..at + 4].copy_from_slice(&rel.to_le_bytes());
                }
                kind => panic!("未対応の再配置: {kind:?}"),
            }
        }
    }
    let entry = addr(*funcs.last().unwrap());
    let mut h = Vec::with_capacity(CODE_START);
    h.extend([0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    h.extend([0u8; 8]);
    h.extend(2u16.to_le_bytes()); // 実行ファイル
    h.extend(0x3eu16.to_le_bytes()); // x86-64
    h.extend(1u32.to_le_bytes());
    h.extend(entry.to_le_bytes());
    h.extend(64u64.to_le_bytes()); // プログラムヘッダーの位置
    h.extend(0u64.to_le_bytes()); // セクションヘッダーは無い
    h.extend(0u32.to_le_bytes());
    h.extend(64u16.to_le_bytes());
    h.extend(56u16.to_le_bytes());
    h.extend(2u16.to_le_bytes());
    h.extend([0u8; 6]);
    // PT_LOAD: 機械語と文字列は RX（5）、__static は RW（6）。
    for (flags, offset, size) in [(5u32, 0, rx_size), (6, rw_start, datas[0].len())] {
        h.extend(1u32.to_le_bytes());
        h.extend(flags.to_le_bytes());
        h.extend((offset as u64).to_le_bytes());
        h.extend(addr(offset).to_le_bytes());
        h.extend(addr(offset).to_le_bytes());
        h.extend((size as u64).to_le_bytes());
        h.extend((size as u64).to_le_bytes());
        h.extend((PAGE_SIZE as u64).to_le_bytes());
    }
    out[..h.len()].copy_from_slice(&h);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elf_separates_rx_and_rw_segments() {
        // 機械語と文字列がページ境界の手前・境界上・複数ページになる場合。
        for code_size in [32, PAGE_SIZE - CODE_START - 24, 2 * PAGE_SIZE] {
            let codes = [Code {
                bytes: vec![0xcc; code_size],
                relocs: vec![
                    (0, Reloc::Abs8, DATA, 0, 0),
                    (8, Reloc::Abs8, DATA, 1, 0),
                    (16, Reloc::X86PCRel4, DATA, 0, -4),
                ],
            }];
            let datas = [vec![0; 64], vec![0x61; 24]];
            let elf = link(&codes, &datas);
            let u16_at = |at| u16::from_le_bytes(elf[at..at + 2].try_into().unwrap());
            let u32_at = |at| u32::from_le_bytes(elf[at..at + 4].try_into().unwrap());
            let u64_at = |at| u64::from_le_bytes(elf[at..at + 8].try_into().unwrap());
            assert_eq!(&elf[..4], b"\x7fELF");
            assert_eq!(u16_at(54), 56);
            assert_eq!(u16_at(56), 2);
            let ph = u64_at(32) as usize;
            let rx_size = u64_at(ph + 32) as usize;
            let rw_start = u64_at(ph + 56 + 8) as usize;
            assert_eq!(rx_size, CODE_START + code_size + datas[1].len());
            assert_eq!(rw_start, rx_size.next_multiple_of(PAGE_SIZE));
            for (at, flags, offset, size) in [
                (ph, 5, 0, rx_size),
                (ph + 56, 6, rw_start, datas[0].len()),
            ] {
                assert_eq!(u32_at(at), 1);
                assert_eq!(u32_at(at + 4), flags);
                assert_eq!(u64_at(at + 8), offset as u64);
                assert_eq!(u64_at(at + 16), BASE + offset as u64);
                assert_eq!(u64_at(at + 24), BASE + offset as u64);
                assert_eq!(u64_at(at + 32), size as u64);
                assert_eq!(u64_at(at + 40), size as u64);
                assert_eq!(u64_at(at + 48), PAGE_SIZE as u64);
            }
            assert_eq!(u64_at(24), BASE + CODE_START as u64);
            assert_eq!(u64_at(CODE_START), BASE + rw_start as u64);
            assert_eq!(
                u64_at(CODE_START + 8),
                BASE + (CODE_START + code_size) as u64
            );
            let rel = i32::from_le_bytes(
                elf[CODE_START + 16..CODE_START + 20].try_into().unwrap(),
            );
            assert_eq!(rel as i64, rw_start as i64 - (CODE_START + 20) as i64);
            assert_eq!(&elf[CODE_START + code_size..rx_size], &datas[1]);
            assert_eq!(&elf[rw_start..], &datas[0]);
        }
    }
}
