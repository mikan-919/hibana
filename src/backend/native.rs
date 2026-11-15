// 構造化 IR を Cranelift で機械語にし、libc を使わない静的な ELF の実行ファイルを作る。
// リンカーは使わない。関数とデータを 1 つの区画に並べ、再配置を自分で埋める。
// ponytail: x86-64 の Linux だけ。aarch64 は syscall の機械語（SYSCALL・START）を足せば動く。macOS・Windows は OS のライブラリを通す別の出力が要る
use crate::error::R;
use crate::ir::{Block, Expr, Node, Op, Program, Stmt, ValueType};
use cranelift_codegen::binemit::Reloc;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::types::I64;
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
// ELF ヘッダー（64 バイト）とプログラムヘッダー 1 つ（56 バイト）の後
const CODE_START: usize = 128;
// 名前の区分。UserExternalName の namespace に入れる
const FUNC: u32 = 0;
const DATA: u32 = 1;

// syscall(番号, a, b, c) -> 結果。System V の引数のレジスタを Linux の syscall のレジスタへ移す
const SYSCALL: &[u8] = &[
    0x48, 0x89, 0xf8, // mov rax, rdi
    0x48, 0x89, 0xf7, // mov rdi, rsi
    0x48, 0x89, 0xd6, // mov rsi, rdx
    0x48, 0x89, 0xca, // mov rdx, rcx
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
const SYS_EXIT: i64 = 60;

struct Code {
    bytes: Vec<u8>,
    // (位置, 種類, 区分, 番号, 加数)
    relocs: Vec<(u32, Reloc, u32, u32, i64)>,
}

// ランタイムの関数の番号。利用者の関数の後に並べる
struct Runtime {
    print_int: u32,
    print_str: u32,
    die: u32,
    syscall: u32,
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
        .ok_or("ネイティブ出力には main が必要")? as u32;
    let mut flags = settings::builder();
    flags.set("opt_level", "speed").unwrap();
    let isa = cranelift_native::builder()
        .map_err(|e| format!("この CPU には未対応: {e}"))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| format!("Cranelift の設定に失敗した: {e}"))?;
    let n = program.functions.len() as u32;
    let rt = Runtime {
        print_int: n,
        print_str: n + 1,
        die: n + 2,
        syscall: n + 3,
    };
    let mut g = Gen {
        isa,
        ctx: Context::new(),
        fctx: FunctionBuilderContext::new(),
        indices: program
            .functions
            .iter()
            .enumerate()
            .map(|(i, f)| (f.name.clone(), (i as u32, f.result_type)))
            .collect(),
        literals: HashMap::new(),
        strings: Vec::new(),
        rt,
    };
    let overflow = g.literal("実行時エラー: オーバーフロー\n");
    let div_zero = g.literal("実行時エラー: 0 で除算した\n");
    for f in &program.functions {
        f.each(&mut |n| {
            if let Node::Expr(Expr::Str(s)) = n {
                g.literal(s);
            }
        });
    }
    let mut codes = Vec::new();
    for f in &program.functions {
        let func = g.function(f, overflow, div_zero);
        codes.push(g.compile(func)?);
    }
    for func in [g.print_int(), g.print_str(), g.die()] {
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
    Ok(link(&codes, &g.strings))
}

struct Gen {
    isa: OwnedTargetIsa,
    ctx: Context,
    fctx: FunctionBuilderContext,
    indices: HashMap<String, (u32, ValueType)>,
    literals: HashMap<String, u32>,
    // 文字列は [長さ: 8 バイト][UTF-8 のバイト列] の形で置く
    strings: Vec<String>,
    rt: Runtime,
}

fn signature(params: usize, result: bool) -> Signature {
    let mut sig = Signature::new(CallConv::SystemV);
    sig.params = vec![AbiParam::new(I64); params];
    if result {
        sig.returns.push(AbiParam::new(I64));
    }
    sig
}

impl Gen {
    fn literal(&mut self, s: &str) -> u32 {
        if let Some(i) = self.literals.get(s) {
            return *i;
        }
        self.strings.push(s.to_string());
        self.literals
            .insert(s.to_string(), self.strings.len() as u32 - 1);
        self.strings.len() as u32 - 1
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

    fn function(&mut self, f: &crate::ir::Function, overflow: u32, div_zero: u32) -> Function {
        let unit = f.result_type == ValueType::Unit;
        let mut func = Function::with_name_signature(
            UserFuncName::default(),
            signature(f.param_types.len(), !unit),
        );
        let mut b = FunctionBuilder::new(&mut func, &mut self.fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let vars: Vec<Variable> = (0..f.locals).map(|_| b.declare_var(I64)).collect();
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
            rt: &self.rt,
            overflow,
            div_zero,
            unit,
        };
        e.stmts(&f.body);
        if unit {
            e.b.ins().return_(&[]);
        } else {
            e.b.ins().trap(TrapCode::unwrap_user(1));
        }
        e.b.seal_all_blocks();
        e.b.finalize(self.isa.frontend_config());
        func
    }

    fn runtime(
        &mut self,
        params: usize,
        build: impl FnOnce(&mut FunctionBuilder, &[Value], FuncRef),
    ) -> Function {
        let config = self.isa.frontend_config();
        let mut func =
            Function::with_name_signature(UserFuncName::default(), signature(params, false));
        let mut b = FunctionBuilder::new(&mut func, &mut self.fctx);
        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let syscall = import(&mut b, FUNC, self.rt.syscall, 4, true);
        let params = b.block_params(entry).to_vec();
        build(&mut b, &params, syscall);
        b.seal_all_blocks();
        b.finalize(config);
        func
    }

    // 整数を 10 進で書き、改行を付けて標準出力に書く
    fn print_int(&mut self) -> Function {
        self.runtime(1, |b, params, syscall| {
            let mf = MemFlagsData::trusted();
            let n = params[0];
            let slot =
                b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 32, 0));
            let buf = b.ins().stack_addr(I64, slot, 0);
            let newline = b.ins().iconst(I64, b'\n' as i64);
            b.ins().istore8(mf, newline, buf, 31);
            let negative = b.ins().icmp_imm_s(IntCC::SignedLessThan, n, 0);
            // 符号を外した値を符号なしで割る。i64::MIN もそのまま扱える
            let neg = b.ins().ineg(n);
            let u = b.ins().select(negative, neg, n);
            let (lp, done) = (b.create_block(), b.create_block());
            b.append_block_param(lp, I64);
            b.append_block_param(lp, I64);
            b.append_block_param(done, I64);
            let pos = b.ins().iconst(I64, 31);
            b.ins().jump(lp, &[u.into(), pos.into()]);
            b.switch_to_block(lp);
            let (u, pos) = (b.block_params(lp)[0], b.block_params(lp)[1]);
            let pos = b.ins().iadd_imm_s(pos, -1);
            let digit = b.ins().urem_imm_u(u, 10);
            let ch = b.ins().iadd_imm_s(digit, b'0' as i64);
            let at = b.ins().iadd(buf, pos);
            b.ins().istore8(mf, ch, at, 0);
            let u = b.ins().udiv_imm_u(u, 10);
            b.ins()
                .brif(u, lp, &[u.into(), pos.into()], done, &[pos.into()]);
            b.switch_to_block(done);
            let pos = b.block_params(done)[0];
            // 先頭の数字の前に '-' を書いておき、負の時だけ出力に含める（19 桁でも位置は 11 以上）
            let minus_pos = b.ins().iadd_imm_s(pos, -1);
            let minus_at = b.ins().iadd(buf, minus_pos);
            let minus = b.ins().iconst(I64, b'-' as i64);
            b.ins().istore8(mf, minus, minus_at, 0);
            let pos = b.ins().select(negative, minus_pos, pos);
            let at = b.ins().iadd(buf, pos);
            let end = b.ins().iconst(I64, 32);
            let len = b.ins().isub(end, pos);
            let (nr, fd) = (b.ins().iconst(I64, SYS_WRITE), b.ins().iconst(I64, 1));
            b.ins().call(syscall, &[nr, fd, at, len]);
            b.ins().return_(&[]);
        })
    }

    // 文字列と改行を標準出力に書く
    fn print_str(&mut self) -> Function {
        self.runtime(1, |b, params, syscall| {
            let mf = MemFlagsData::trusted();
            let s = params[0];
            write(b, syscall, 1, s);
            let slot =
                b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 1, 0));
            let buf = b.ins().stack_addr(I64, slot, 0);
            let newline = b.ins().iconst(I64, b'\n' as i64);
            b.ins().istore8(mf, newline, buf, 0);
            let (nr, fd, len) = (
                b.ins().iconst(I64, SYS_WRITE),
                b.ins().iconst(I64, 1),
                b.ins().iconst(I64, 1),
            );
            b.ins().call(syscall, &[nr, fd, buf, len]);
            b.ins().return_(&[]);
        })
    }

    // 文字列を標準エラーに書き、終了コード 1 で終わる
    fn die(&mut self) -> Function {
        self.runtime(1, |b, params, syscall| {
            write(b, syscall, 2, params[0]);
            let (nr, code, z) = (
                b.ins().iconst(I64, SYS_EXIT),
                b.ins().iconst(I64, 1),
                b.ins().iconst(I64, 0),
            );
            b.ins().call(syscall, &[nr, code, z, z]);
            b.ins().trap(TrapCode::unwrap_user(1));
        })
    }
}

fn write(b: &mut FunctionBuilder, syscall: FuncRef, fd: i64, s: Value) {
    let mf = MemFlagsData::trusted();
    let len = b.ins().load(I64, mf, s, 0);
    let bytes = b.ins().iadd_imm_s(s, 8);
    let (nr, fd) = (b.ins().iconst(I64, SYS_WRITE), b.ins().iconst(I64, fd));
    b.ins().call(syscall, &[nr, fd, bytes, len]);
}

fn import(
    b: &mut FunctionBuilder,
    namespace: u32,
    index: u32,
    params: usize,
    result: bool,
) -> FuncRef {
    let signature = b.import_signature(signature(params, result));
    let name = b
        .func
        .declare_imported_user_function(UserExternalName::new(namespace, index));
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
    funcs: HashMap<u32, FuncRef>,
    indices: &'a HashMap<String, (u32, ValueType)>,
    literals: &'a HashMap<String, u32>,
    rt: &'a Runtime,
    overflow: u32,
    div_zero: u32,
    unit: bool,
}

impl Body<'_, '_> {
    fn func(&mut self, index: u32, params: usize, result: bool) -> FuncRef {
        if let Some(f) = self.funcs.get(&index) {
            return *f;
        }
        let f = import(&mut self.b, FUNC, index, params, result);
        self.funcs.insert(index, f);
        f
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

    // 条件が真なら、メッセージを出して終了する
    fn die_if(&mut self, cond: Value, message: u32) {
        let (fail, ok) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(cond, fail, &[], ok, &[]);
        self.b.switch_to_block(fail);
        self.b.set_cold_block(fail);
        let msg = self.data(message);
        let die = self.func(self.rt.die, 1, false);
        self.b.ins().call(die, &[msg]);
        self.b.ins().trap(TrapCode::unwrap_user(1));
        self.b.switch_to_block(ok);
    }

    // return の後の到達しない位置に、続きを書くための区画を作る
    fn unreachable(&mut self) {
        let next = self.b.create_block();
        self.b.switch_to_block(next);
    }

    fn stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::Set(i, e) => {
                    let v = self.expr(e);
                    self.b.def_var(self.vars[*i as usize], v);
                }
                Stmt::Eval(e) => {
                    self.expr(e);
                }
                Stmt::Print(t, e) => {
                    let v = self.expr(e);
                    let f = if *t == ValueType::Int {
                        self.rt.print_int
                    } else {
                        self.rt.print_str
                    };
                    let f = self.func(f, 1, false);
                    self.b.ins().call(f, &[v]);
                }
                Stmt::While(c, body) => {
                    let (head, inner, exit) = (
                        self.b.create_block(),
                        self.b.create_block(),
                        self.b.create_block(),
                    );
                    self.b.ins().jump(head, &[]);
                    self.b.switch_to_block(head);
                    let c = self.expr(c);
                    self.b.ins().brif(c, inner, &[], exit, &[]);
                    self.b.switch_to_block(inner);
                    self.stmts(body);
                    self.b.ins().jump(head, &[]);
                    self.b.switch_to_block(exit);
                }
                Stmt::Return(e) => {
                    let v = e.as_ref().map(|e| self.expr(e));
                    match v {
                        Some(v) if !self.unit => self.b.ins().return_(&[v]),
                        _ => self.b.ins().return_(&[]),
                    };
                    self.unreachable();
                }
            }
        }
    }

    fn block(&mut self, b: &Block) -> Value {
        self.stmts(&b.stmts);
        self.expr(&b.value)
    }

    fn expr(&mut self, e: &Expr) -> Value {
        match e {
            Expr::Int(n) => self.b.ins().iconst(I64, *n),
            Expr::Str(s) => self.data(self.literals[s]),
            Expr::Local(i) => self.b.use_var(self.vars[*i as usize]),
            Expr::Not(a) => {
                let a = self.expr(a);
                let c = self.b.ins().icmp_imm_s(IntCC::Equal, a, 0);
                self.b.ins().uextend(I64, c)
            }
            Expr::Call(name, args) => {
                let (index, result) = self.indices[name];
                let args: Vec<Value> = args.iter().map(|a| self.expr(a)).collect();
                let unit = result == ValueType::Unit;
                let f = self.func(index, args.len(), !unit);
                let call = self.b.ins().call(f, &args);
                match self.b.inst_results(call).first() {
                    Some(v) => *v,
                    None => self.b.ins().iconst(I64, 0),
                }
            }
            Expr::If(c, t, f) => {
                let c = self.expr(c);
                let (then, other, merge) = (
                    self.b.create_block(),
                    self.b.create_block(),
                    self.b.create_block(),
                );
                self.b.append_block_param(merge, I64);
                self.b.ins().brif(c, then, &[], other, &[]);
                self.b.switch_to_block(then);
                let v = self.block(t);
                self.b.ins().jump(merge, &[v.into()]);
                self.b.switch_to_block(other);
                let v = self.block(f);
                self.b.ins().jump(merge, &[v.into()]);
                self.b.switch_to_block(merge);
                self.b.block_params(merge)[0]
            }
            Expr::Block(b) => self.block(b),
            Expr::Binary(op, a, b) => {
                let (a, b) = (self.expr(a), self.expr(b));
                let cmp = match op {
                    Op::Eq => Some(IntCC::Equal),
                    Op::Ne => Some(IntCC::NotEqual),
                    Op::Lt => Some(IntCC::SignedLessThan),
                    Op::Le => Some(IntCC::SignedLessThanOrEqual),
                    Op::Gt => Some(IntCC::SignedGreaterThan),
                    Op::Ge => Some(IntCC::SignedGreaterThanOrEqual),
                    _ => None,
                };
                if let Some(cc) = cmp {
                    let c = self.b.ins().icmp(cc, a, b);
                    return self.b.ins().uextend(I64, c);
                }
                match op {
                    Op::Add | Op::Sub | Op::Mul => {
                        let (r, of) = match op {
                            Op::Add => self.b.ins().sadd_overflow(a, b),
                            Op::Sub => self.b.ins().ssub_overflow(a, b),
                            _ => self.b.ins().smul_overflow(a, b),
                        };
                        self.die_if(of, self.overflow);
                        r
                    }
                    _ => {
                        let zero = self.b.ins().icmp_imm_s(IntCC::Equal, b, 0);
                        self.die_if(zero, self.div_zero);
                        let min = self.b.ins().icmp_imm_s(IntCC::Equal, a, i64::MIN);
                        let minus_one = self.b.ins().icmp_imm_s(IntCC::Equal, b, -1);
                        let of = self.b.ins().band(min, minus_one);
                        self.die_if(of, self.overflow);
                        if *op == Op::Div {
                            self.b.ins().sdiv(a, b)
                        } else {
                            self.b.ins().srem(a, b)
                        }
                    }
                }
            }
        }
    }
}

// 関数とデータを並べて再配置を埋め、ELF を書く
fn link(codes: &[Code], strings: &[String]) -> Vec<u8> {
    let mut out = vec![0u8; CODE_START];
    let mut funcs = Vec::new();
    for c in codes {
        while out.len() % 16 != 0 {
            out.push(0xcc);
        }
        funcs.push(out.len());
        out.extend(&c.bytes);
    }
    let mut datas = Vec::new();
    for s in strings {
        while out.len() % 8 != 0 {
            out.push(0);
        }
        datas.push(out.len());
        out.extend((s.len() as u64).to_le_bytes());
        out.extend(s.as_bytes());
    }
    let addr = |offset: usize| BASE + offset as u64;
    for (c, start) in codes.iter().zip(&funcs) {
        for &(offset, kind, namespace, index, addend) in &c.relocs {
            let target = if namespace == FUNC {
                funcs[index as usize]
            } else {
                datas[index as usize]
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
    let size = out.len() as u64;
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
    h.extend(1u16.to_le_bytes());
    h.extend([0u8; 6]);
    // 読み込み可能な区画 1 つ。読み取りと実行を許す
    h.extend(1u32.to_le_bytes());
    h.extend(5u32.to_le_bytes());
    h.extend(0u64.to_le_bytes());
    h.extend(BASE.to_le_bytes());
    h.extend(BASE.to_le_bytes());
    h.extend(size.to_le_bytes());
    h.extend(size.to_le_bytes());
    h.extend(0x1000u64.to_le_bytes());
    out[..h.len()].copy_from_slice(&h);
    out
}
