// 構造化 IR を WebAssembly に変換する。
// IR の値（型の番号と中身）は 2 つの i64 で表し、関数は 2 つの i64 を返す。
// 公開関数と外部関数の境界では、int と number は f64、str は (長さ << 32) | 位置 の i64 に変換する。
// メモリ: [0, 8) は使わない、[8, 72) はランタイムの状態（runtime.hb の __static）、その後に文字列のリテラル、ヒープ。
use crate::ir::{self, Block, Expr, Node, Prim, Program, Stmt, ValueType};
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ElementSection, Elements, EntityType,
    ExportKind, ExportSection, Function, FunctionSection, ImportSection, Instruction as I, MemArg,
    MemorySection, MemoryType, Module, RefType, TableSection, TableType, TypeSection, ValType,
};

const STATIC: i64 = 8;
const FAIL_AT: i64 = 24;
const LITERALS: u32 = 72;

pub fn emit(program: &Program) -> Vec<u8> {
    emit_selected(program, None)
}

fn mem(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

// 型の並び。同じ型は 1 つにまとめる
#[derive(Default)]
struct Types {
    section: TypeSection,
    seen: HashMap<(Vec<ValType>, Vec<ValType>), u32>,
}

impl Types {
    fn get(&mut self, params: u32, results: u32) -> u32 {
        self.signature(vec![ValType::I64; params as usize], vec![ValType::I64; results as usize])
    }
    fn signature(&mut self, params: Vec<ValType>, results: Vec<ValType>) -> u32 {
        let n = self.seen.len() as u32;
        *self.seen.entry((params.clone(), results.clone())).or_insert_with(|| {
            self.section.ty().function(params, results);
            n
        })
    }
    fn boundary(&mut self, params: &[ValueType], result: ValueType) -> u32 {
        let ty = |t: ValueType| if matches!(t, ValueType::Int | ValueType::Number) { ValType::F64 } else { ValType::I64 };
        self.signature(params.iter().copied().map(ty).collect(), if result == ValueType::Unit { vec![] } else { vec![ty(result)] })
    }
}

// 公開する関数は selected、None なら境界で渡せる利用者の関数すべて
pub fn emit_selected(program: &Program, selected: Option<&[String]>) -> Vec<u8> {
    // 文字列のリテラルを並べる。[長さ][バイト列] を 8 バイト境界に置く
    let mut literals: HashMap<String, i64> = HashMap::new();
    let mut data = Vec::new();
    let mut writes = false;
    let mut now = false;
    let mut reads = false;
    let mut table: Vec<&str> = Vec::new();
    for f in &program.functions {
        f.each(&mut |n| match n {
            Node::Expr(Expr::Str(s)) if !literals.contains_key(s) => {
                while data.len() % 8 != 0 {
                    data.push(0);
                }
                literals.insert(s.clone(), LITERALS as i64 + data.len() as i64);
                data.extend((s.len() as u64).to_le_bytes());
                data.extend(s.as_bytes());
            }
            Node::Expr(Expr::Prim(Prim::Write, _)) => writes = true,
            Node::Expr(Expr::Prim(Prim::Now, _)) => now = true,
            Node::Expr(Expr::Prim(Prim::Read, _)) => reads = true,
            Node::Expr(Expr::Func(name)) if !table.contains(&name.as_str()) => table.push(name),
            _ => {}
        });
    }
    while data.len() % 8 != 0 {
        data.push(0);
    }
    let heap = LITERALS as u64 + data.len() as u64;
    let pages = heap.div_ceil(65536).max(1);
    let exported: Vec<&ir::Function> = program
        .functions
        .iter()
        .filter(|f| match selected {
            Some(names) => names.contains(&f.name),
            // 文字列を渡す関数は、変換するランタイムの関数がある時だけ公開する
            None => {
                f.boundary()
                    && (!(f.param_types.contains(&ValueType::Str)
                        || f.result_type == ValueType::Str)
                        || program.functions.iter().any(|g| g.name == "__str_in"))
            }
        })
        .collect();
    let has_str =
        |ts: &[ValueType], r: ValueType| ts.iter().chain([&r]).any(|t| matches!(t, ValueType::Str));
    let strings = exported
        .iter()
        .any(|f| has_str(&f.param_types, f.result_type))
        || program
            .imports
            .iter()
            .any(|f| has_str(&f.param_types, f.result_type));

    let mut types = Types::default();
    let mut imports = ImportSection::new();
    let mut count = 0u32;
    let mut import = |types: &mut Types, module: &str, name: &str, params: u32, results: u32| {
        imports.import(
            module,
            name,
            EntityType::Function(types.get(params, results)),
        );
        count += 1;
        count - 1
    };
    let write = writes.then(|| import(&mut types, "hibana", "write", 3, 0));
    let now = now.then(|| import(&mut types, "hibana", "now_ms", 0, 1));
    let read = reads.then(|| import(&mut types, "hibana", "read", 3, 1));
    let mut hosts = Vec::new();
    for f in &program.imports {
        imports.import("hibana_host", &f.name, EntityType::Function(types.boundary(&f.param_types, f.result_type)));
        hosts.push(count);
        count += 1;
    }
    let imported = count;
    // 定義する関数の番号: IR の関数、外部関数の受け口、公開関数の入口、確保と初期化
    let mut indices: HashMap<&str, u32> = HashMap::new();
    for f in &program.functions {
        indices.insert(&f.name, count);
        count += 1;
    }
    for f in &program.imports {
        indices.insert(&f.name, count);
        count += 1;
    }
    let wrappers = count;
    count += exported.len() as u32;
    let alloc = count;

    let mut functions = FunctionSection::new();
    let mut code = CodeSection::new();
    let block_type = types.get(0, 2);
    let mut cx = Cx {
        indices: &indices,
        literals: &literals,
        table: &table,
        write,
        now,
        read,
        block_type,
    };
    for f in &program.functions {
        functions.function(types.get(2 * f.params, 2));
        let mut e = Emit {
            out: Vec::new(),
            locals: 2 * f.locals,
            cx: &mut cx,
            types: &mut types,
            depth: 0,
            catches: Vec::new(),
        };
        e.stmts(&f.body);
        e.out.extend([I::I64Const(0), I::I64Const(0), I::End]);
        let extra = e.locals - 2 * f.params;
        code.function(&function(&[(extra, ValType::I64)], &e.out));
    }
    // 外部関数の受け口。値を境界の形に変えて呼ぶ
    for (f, host) in program.imports.iter().zip(&hosts) {
        functions.function(types.get(2 * f.param_types.len() as u32, 2));
        let t = 2 * f.param_types.len() as u32;
        let mut out = Vec::new();
        for (i, p) in f.param_types.iter().enumerate() {
            let i = 2 * i as u32;
            match p {
                ValueType::Str => out.extend([
                    I::LocalGet(i),
                    I::LocalGet(i + 1),
                    I::Call(indices["__str_out"]),
                    I::LocalSet(t),
                    I::Drop,
                    I::LocalGet(t),
                ]),
                ValueType::Number => out.extend([I::LocalGet(i + 1), I::F64ReinterpretI64]),
                ValueType::Int => out.extend([I::LocalGet(i + 1), I::F64ConvertI64S]),
                _ => out.push(I::LocalGet(i + 1)),
            }
        }
        out.push(I::Call(*host));
        out.extend(from_raw(f.result_type, t, &indices));
        out.push(I::End);
        code.function(&function(&[(1, ValType::I64)], &out));
    }
    // 公開関数の入口。境界の形から値に変えて呼ぶ
    let mut exports = ExportSection::new();
    for (k, f) in exported.iter().enumerate() {
        let n = f.param_types.len() as u32;
        functions.function(types.boundary(&f.param_types, f.result_type));
        exports.export(&f.name, ExportKind::Func, wrappers + k as u32);
        let mut out = Vec::new();
        for (i, p) in f.param_types.iter().enumerate() {
            if matches!(p, ValueType::Int | ValueType::Number) {
                out.extend([I::I64Const(ir::NUM), I::LocalGet(i as u32), I::I64ReinterpretF64, I::Call(indices[if *p == ValueType::Int { "__int_in" } else { "__num_checked" }])]);
            } else { out.extend(from_raw(*p, i as u32, &indices).into_iter().skip(1)); }
        }
        out.push(I::Call(indices[f.name.as_str()]));
        match f.result_type {
            ValueType::Unit => out.extend([I::Drop, I::Drop]),
            ValueType::Str => out.extend([
                I::Call(indices["__str_out"]),
                I::LocalSet(n),
                I::Drop,
                I::LocalGet(n),
            ]),
            ValueType::Number => out.extend([I::LocalSet(n), I::Drop, I::LocalGet(n), I::F64ReinterpretI64]),
            ValueType::Int => out.extend([I::LocalSet(n), I::Drop, I::LocalGet(n), I::F64ConvertI64S]),
            _ => out.extend([I::LocalSet(n), I::Drop, I::LocalGet(n)]),
        }
        out.push(I::End);
        code.function(&function(&[(1, ValType::I64)], &out));
    }
    if strings {
        // JavaScript の接続コードが文字列を書く領域を確保する。リセットは確保の状態を最初に戻す
        functions.function(types_i32(&mut types));
        let mut out = vec![I::I64Const(ir::INT), I::LocalGet(0), I::I64ExtendI32U];
        out.extend([
            I::Call(indices["__alloc"]),
            I::LocalSet(1),
            I::Drop,
            I::LocalGet(1),
            I::I32WrapI64,
            I::End,
        ]);
        let mut f = Function::new([(1, ValType::I64)]);
        for i in &out {
            f.instruction(i);
        }
        code.function(&f);
        functions.function(types.get(0, 0));
        let reset = [
            I::I32Const(STATIC as i32),
            I::I64Const(heap as i64),
            I::I64Store(mem(0, 3)),
            I::I32Const(STATIC as i32 + 8),
            I::MemorySize(0),
            I::I64ExtendI32U,
            I::I64Const(16),
            I::I64Shl,
            I::I64Store(mem(0, 3)),
            I::End,
        ];
        code.function(&function(&[], &reset));
        exports.export("__hibana_alloc", ExportKind::Func, alloc);
        exports.export("__hibana_reset", ExportKind::Func, alloc + 1);
    }
    if strings || writes || reads || selected.is_none() {
        exports.export("memory", ExportKind::Memory, 0);
    }
    let mut memory = MemorySection::new();
    memory.memory(MemoryType {
        minimum: pages,
        maximum: Some(65536),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut init = Vec::new();
    init.extend(heap.to_le_bytes());
    init.extend((pages * 65536).to_le_bytes());
    // 実行時エラーの文字列、中断、処理中の with は 0 から始める
    init.extend([0u8; 48]);
    init.extend(data);
    let mut datas = DataSection::new();
    datas.active(0, &ConstExpr::i32_const(STATIC as i32), init);
    let mut tables = TableSection::new();
    let mut elements = ElementSection::new();
    if !table.is_empty() {
        tables.table(TableType {
            element_type: RefType::FUNCREF,
            table64: false,
            minimum: table.len() as u64,
            maximum: Some(table.len() as u64),
            shared: false,
        });
        let funcs: Vec<u32> = table.iter().map(|n| indices[n]).collect();
        elements.active(
            Some(0),
            &ConstExpr::i32_const(0),
            Elements::Functions(funcs.into()),
        );
    }
    let mut module = Module::new();
    module.section(&types.section);
    if imported > 0 {
        module.section(&imports);
    }
    module.section(&functions);
    if !table.is_empty() {
        module.section(&tables);
    }
    module.section(&memory);
    module.section(&exports);
    if !table.is_empty() {
        module.section(&elements);
    }
    module.section(&code);
    module.section(&datas);
    module.finish()
}

fn types_i32(types: &mut Types) -> u32 {
    types.signature(vec![ValType::I32], vec![ValType::I32])
}

// 境界の形の値（局所変数 at）を IR の値にする。先頭は捨ててよい LocalGet ではなく型の番号
fn from_raw(t: ValueType, at: u32, indices: &HashMap<&str, u32>) -> Vec<I<'static>> {
    match t {
        ValueType::Unit => vec![I::Nop, I::I64Const(ir::UNIT), I::I64Const(0)],
        ValueType::Str => vec![
            I::LocalSet(at),
            I::I64Const(ir::INT),
            I::LocalGet(at),
            I::Call(indices["__str_in"]),
        ],
        ValueType::Int => vec![I::I64ReinterpretF64, I::LocalSet(at), I::I64Const(ir::NUM), I::LocalGet(at), I::Call(indices["__int_in"])],
        ValueType::Number => vec![I::I64ReinterpretF64, I::LocalSet(at), I::I64Const(ir::NUM), I::LocalGet(at), I::Call(indices["__num_checked"])],
        ValueType::Bool => vec![I::LocalSet(at), I::I64Const(ir::BOOL), I::LocalGet(at)],
        _ => vec![I::LocalSet(at), I::I64Const(ir::INT), I::LocalGet(at)],
    }
}

fn function(locals: &[(u32, ValType)], body: &[I<'static>]) -> Function {
    let mut f = Function::new(locals.iter().copied());
    for i in body {
        f.instruction(i);
    }
    f
}

struct Cx<'a> {
    indices: &'a HashMap<&'a str, u32>,
    literals: &'a HashMap<String, i64>,
    table: &'a [&'a str],
    write: Option<u32>,
    read: Option<u32>,
    now: Option<u32>,
    block_type: u32,
}

struct Emit<'a, 'b> {
    out: Vec<I<'static>>,
    // 局所変数（値 1 つに 2 つ）と演算用の一時変数を合わせた数
    locals: u32,
    cx: &'b mut Cx<'a>,
    types: &'b mut Types,
    // 入れ子になった block・loop・if の数と、Catch の位置（br の深さを数える）
    depth: u32,
    catches: Vec<u32>,
}

impl Emit<'_, '_> {
    fn temp(&mut self) -> u32 {
        self.locals += 1;
        self.locals - 1
    }

    fn call(&mut self, name: &str) {
        self.out.push(I::Call(self.cx.indices[name]));
    }

    // 失敗した時にランタイムの関数を呼ぶ。その関数は戻らない
    fn fail_if(&mut self, name: &str) {
        self.out.push(I::If(BlockType::Empty));
        self.call(name);
        self.out.extend([I::Unreachable, I::End]);
    }

    fn stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::Set(i, e) => {
                    self.expr(e);
                    self.out
                        .extend([I::LocalSet(2 * i + 1), I::LocalSet(2 * i)]);
                }
                Stmt::Eval(e) => {
                    self.expr(e);
                    self.out.extend([I::Drop, I::Drop]);
                }
                Stmt::While(c, body) => {
                    self.out.push(I::Block(BlockType::Empty));
                    self.out.push(I::Loop(BlockType::Empty));
                    self.depth += 2;
                    self.payload(c);
                    self.out.extend([I::I64Eqz, I::BrIf(1)]);
                    self.stmts(body);
                    self.depth -= 2;
                    self.out.extend([I::Br(0), I::End, I::End]);
                }
                Stmt::Return(e) => {
                    self.expr(e);
                    self.out.push(I::Return);
                }
                Stmt::Escape => {
                    let target = *self.catches.last().expect("Escape は Catch の中だけ");
                    self.out.extend([
                        I::I64Const(ir::UNIT),
                        I::I64Const(0),
                        I::Br(self.depth - target),
                    ]);
                }
            }
        }
    }

    fn block(&mut self, b: &Block) {
        self.stmts(&b.stmts);
        self.expr(&b.value);
    }

    // 値の中身だけを積む
    fn payload(&mut self, e: &Expr) {
        if let Expr::Const(_, n) = e {
            self.out.push(I::I64Const(*n));
            return;
        }
        if let Expr::Local(i) = e {
            self.out.push(I::LocalGet(2 * i + 1));
            return;
        }
        self.expr(e);
        let t = self.temp();
        self.out.extend([I::LocalSet(t), I::Drop, I::LocalGet(t)]);
    }

    // 中身を一時変数に置く
    fn payload_temp(&mut self, e: &Expr) -> u32 {
        if let Expr::Local(i) = e {
            return 2 * i + 1;
        }
        self.payload(e);
        let t = self.temp();
        self.out.push(I::LocalSet(t));
        t
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Const(t, n) => self.out.extend([I::I64Const(*t), I::I64Const(*n)]),
            Expr::Str(s) => self
                .out
                .extend([I::I64Const(ir::STR), I::I64Const(self.cx.literals[s])]),
            Expr::Local(i) => self
                .out
                .extend([I::LocalGet(2 * i), I::LocalGet(2 * i + 1)]),
            Expr::Func(n) => {
                let k = self.cx.table.iter().position(|x| x == n).unwrap();
                self.out
                    .extend([I::I64Const(ir::INT), I::I64Const(k as i64)]);
            }
            Expr::Call(name, args) => {
                for a in args {
                    self.expr(a);
                }
                self.call(name);
            }
            Expr::CallIndirect(g, args) => {
                self.expr(g);
                let (gt, gp) = (self.temp(), self.temp());
                self.out
                    .extend([I::LocalSet(gp), I::LocalTee(gt), I::LocalGet(gp)]);
                for a in args {
                    self.expr(a);
                }
                let ty = self.types.get(2 + 2 * args.len() as u32, 2);
                self.out.extend([
                    I::LocalGet(gp),
                    I::I32WrapI64,
                    I::I64Load(mem(0, 3)),
                    I::I32WrapI64,
                    I::CallIndirect {
                        type_index: ty,
                        table_index: 0,
                    },
                ]);
            }
            Expr::If(c, t, f) => {
                self.payload(c);
                self.out.extend([
                    I::I64Eqz,
                    I::I32Eqz,
                    I::If(BlockType::FunctionType(self.cx.block_type)),
                ]);
                self.depth += 1;
                self.block(t);
                self.out.push(I::Else);
                self.block(f);
                self.depth -= 1;
                self.out.push(I::End);
            }
            Expr::Block(b) => self.block(b),
            // 抜けた時は Escape が unit を積んで、この塊の終わりへ飛ぶ
            Expr::Catch(b) => {
                self.out
                    .push(I::Block(BlockType::FunctionType(self.cx.block_type)));
                self.depth += 1;
                self.catches.push(self.depth);
                self.block(b);
                self.catches.pop();
                self.depth -= 1;
                self.out.push(I::End);
            }
            Expr::Prim(p, args) => self.prim(*p, args),
        }
    }

    fn prim(&mut self, p: Prim, args: &[Expr]) {
        use Prim::*;
        match p {
            Tag => {
                self.expr(&args[0]);
                let t = self.temp();
                self.out.extend([
                    I::Drop,
                    I::LocalSet(t),
                    I::I64Const(ir::INT),
                    I::LocalGet(t),
                ]);
                return;
            }
            Fail => {
                let m = self.payload_temp(&args[0]);
                self.out.extend([
                    I::I32Const(FAIL_AT as i32),
                    I::LocalGet(m),
                    I::I64Store(mem(0, 3)),
                    I::Unreachable,
                ]);
                return;
            }
            _ => {}
        }
        let xs: Vec<u32> = args.iter().map(|a| self.payload_temp(a)).collect();
        let get = |k: usize| I::LocalGet(xs[k]);
        let addr = |k: usize| [I::LocalGet(xs[k]), I::I32WrapI64];
        let tag = match p {
            MakeV => {
                self.out.extend([get(0), get(1)]);
                return;
            }
            Store | Store8 | Write => ir::UNIT,
            Eq | Ne | Lt | Le | Gt | Ge | Not | FEq | FLt | FLe => ir::BOOL,
            _ => ir::INT,
        };
        match p {
            Store | Store8 => {
                self.out.extend(addr(0));
                self.out.push(get(1));
                self.out.push(if p == Store {
                    I::I64Store(mem(0, 3))
                } else {
                    I::I64Store8(mem(0, 0))
                });
                self.out.extend([I::I64Const(ir::UNIT), I::I64Const(0)]);
                return;
            }
            Write => {
                self.out
                    .extend([get(0), get(1), get(2), I::Call(self.cx.write.unwrap())]);
                self.out.extend([I::I64Const(ir::UNIT), I::I64Const(0)]);
                return;
            }
            Add | Sub | Mul | Div | Rem => {
                let r = self.temp();
                self.arith(p, xs[0], xs[1], r);
                self.out.extend([I::I64Const(ir::INT), I::LocalGet(r)]);
                return;
            }
            _ => {}
        }
        self.out.push(I::I64Const(tag));
        match p {
            Payload => self.out.push(get(0)),
            Eq | Ne | Lt | Le | Gt | Ge => {
                self.out.extend([get(0), get(1)]);
                self.out.extend([
                    match p {
                        Eq => I::I64Eq,
                        Ne => I::I64Ne,
                        Lt => I::I64LtS,
                        Le => I::I64LeS,
                        Gt => I::I64GtS,
                        _ => I::I64GeS,
                    },
                    I::I64ExtendI32U,
                ]);
            }
            Not => self.out.extend([get(0), I::I64Eqz, I::I64ExtendI32U]),
            And | Or | Shl | Shr => {
                self.out.extend([get(0), get(1)]);
                self.out.push(match p {
                    And => I::I64And,
                    Or => I::I64Or,
                    Shl => I::I64Shl,
                    _ => I::I64ShrU,
                });
            }
            Load => {
                self.out.extend(addr(0));
                self.out.push(I::I64Load(mem(0, 3)));
            }
            Load8 => {
                self.out.extend(addr(0));
                self.out.push(I::I64Load8U(mem(0, 0)));
            }
            // 足りないページを増やし、その先頭を返す。増やせなければ止める
            Pages => {
                let t = self.temp();
                self.out.extend([
                    get(0),
                    I::I64Const(65535),
                    I::I64Add,
                    I::I64Const(16),
                    I::I64ShrU,
                    I::I32WrapI64,
                    I::MemoryGrow(0),
                    I::I64ExtendI32S,
                    I::LocalTee(t),
                    I::I64Const(-1),
                    I::I64Eq,
                    I::If(BlockType::Empty),
                    I::Unreachable,
                    I::End,
                    I::LocalGet(t),
                    I::I64Const(16),
                    I::I64Shl,
                ]);
            }
            Static => self.out.push(I::I64Const(STATIC)),
            Now => self.out.push(I::Call(self.cx.now.unwrap())),
            Read => self.out.extend([get(0), get(1), get(2), I::Call(self.cx.read.unwrap())]),
            FFromInt => self.out.extend([get(0), I::F64ConvertI64S, I::I64ReinterpretF64]),
            FToInt | FAdd | FSub | FMul | FDiv | FNeg | FFloor | FSqrt | FEq | FLt | FLe => {
                self.out.extend([get(0), I::F64ReinterpretI64]);
                if matches!(p, FAdd | FSub | FMul | FDiv | FEq | FLt | FLe) { self.out.extend([get(1), I::F64ReinterpretI64]); }
                self.out.push(match p {
                    FToInt => I::I64TruncSatF64S, FAdd => I::F64Add, FSub => I::F64Sub, FMul => I::F64Mul, FDiv => I::F64Div,
                    FNeg => I::F64Neg, FFloor => I::F64Floor, FSqrt => I::F64Sqrt, FEq => I::F64Eq, FLt => I::F64Lt, FLe => I::F64Le, _ => unreachable!(),
                });
                if matches!(p, FEq | FLt | FLe) { self.out.push(I::I64ExtendI32U); }
                else if p != FToInt { self.out.push(I::I64ReinterpretF64); }
            }
            _ => unreachable!(),
        }
    }

    // WASM の整数演算は周回するため、hibana の範囲外と 0 による除算を検査する
    fn arith(&mut self, op: Prim, a: u32, b: u32, r: u32) {
        let (a, b) = (I::LocalGet(a), I::LocalGet(b));
        if matches!(op, Prim::Div | Prim::Rem) {
            self.out.extend([b.clone(), I::I64Eqz]);
            self.fail_if("__div_zero");
            self.out.extend([
                a.clone(),
                I::I64Const(i64::MIN),
                I::I64Eq,
                b.clone(),
                I::I64Const(-1),
                I::I64Eq,
                I::I32And,
            ]);
            self.fail_if("__overflow");
        }
        if op == Prim::Mul {
            // -1 * MIN は割り算で確かめられないので先に調べる
            self.out.extend([
                a.clone(),
                I::I64Const(-1),
                I::I64Eq,
                b.clone(),
                I::I64Const(i64::MIN),
                I::I64Eq,
                I::I32And,
            ]);
            self.fail_if("__overflow");
        }
        self.out.extend([a.clone(), b.clone()]);
        self.out.push(match op {
            Prim::Add => I::I64Add,
            Prim::Sub => I::I64Sub,
            Prim::Mul => I::I64Mul,
            Prim::Div => I::I64DivS,
            _ => I::I64RemS,
        });
        self.out.push(I::LocalSet(r));
        let res = I::LocalGet(r);
        match op {
            Prim::Add | Prim::Sub => {
                let add = op == Prim::Add;
                self.out.extend([
                    a.clone(),
                    if add { res.clone() } else { b.clone() },
                    I::I64Xor,
                    if add { b.clone() } else { a.clone() },
                    res.clone(),
                    I::I64Xor,
                    I::I64And,
                    I::I64Const(0),
                    I::I64LtS,
                ]);
                self.fail_if("__overflow");
            }
            Prim::Mul => {
                self.out.extend([
                    a.clone(),
                    I::I64Eqz,
                    I::I32Eqz,
                    a.clone(),
                    I::I64Const(-1),
                    I::I64Ne,
                    I::I32And,
                    I::If(BlockType::Empty),
                    res,
                    a,
                    I::I64DivS,
                    b,
                    I::I64Ne,
                ]);
                self.fail_if("__overflow");
                self.out.push(I::End);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn node(bytes: &[u8], script: &str) -> std::process::Output {
        let mut child = Command::new("node").args(["-e", script]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        child.wait_with_output().unwrap()
    }

    #[test]
    fn string_memory_grows_resets_and_preserves_literals() {
        let bytes = crate::to_wasm_library("fn literal() -> String { '日本語' }", &["literal".into()])
            .unwrap();
        let script = r#"const x = new WebAssembly.Instance(new WebAssembly.Module(require('fs').readFileSync(0)), {}).exports;
const packed = x.literal(); const ptr = Number(packed & 0xffffffffn), len = Number(packed >> 32n);
const read = () => Buffer.from(x.memory.buffer, ptr, len).toString();
if (read() !== '日本語') throw Error('literal');
const first = x.__hibana_alloc(200000); if (x.memory.buffer.byteLength < first + 200000) throw Error('grow');
let trapped = false; try { x.__hibana_alloc(-1); } catch { trapped = true; } if (!trapped) throw Error('allocation');
x.__hibana_reset(); if (x.__hibana_alloc(1) !== first || x.literal() !== packed || read() !== '日本語') throw Error('reset');"#;
        let result = node(&bytes, script);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    }

    #[test]
    fn library_exports_only_requested_reachable_functions() {
        let used = "fn plus(a: int, b: int) -> int { a + b }\nfn twice(n: int) -> int { plus(n, n) }\nfn result(n: int) -> int { twice(n) + 1 }\n";
        let with_unused = format!(
            "{used}fn unused() -> String {{ 'unused' }}\nfn unused_print() {{ put(999) }}\n"
        );
        let names = vec!["result".to_string()];
        let bytes = crate::to_wasm_library(&with_unused, &names).unwrap();
        assert_eq!(bytes, crate::to_wasm_library(used, &names).unwrap());
        let script = r#"
const m = new WebAssembly.Module(require('fs').readFileSync(0));
if (JSON.stringify(WebAssembly.Module.imports(m)) !== '[]') throw Error('unexpected imports');
const x = new WebAssembly.Instance(m, {}).exports;
if (Object.keys(x).join(',') !== 'result' || x.result(20) !== 41) throw Error('unexpected exports or result');
"#;
        let mut child = Command::new("node")
            .args(["-e", script])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );

        let bytes = crate::to_wasm_library(used, &["result".into(), "plus".into()]).unwrap();
        let result = node(&bytes, r#"const m = new WebAssembly.Module(require('fs').readFileSync(0));
const x = new WebAssembly.Instance(m, {}).exports;
if (Object.keys(x).join(',') !== 'plus,result' || x.plus(20, 22) !== 42) throw Error('exports');"#);
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));

        assert!(
            crate::to_wasm_library(used, &["missing".into()])
                .unwrap_err()
                .contains("無い")
        );
        assert!(
            crate::to_wasm_library(used, &["result".into(), "result".into()])
                .unwrap_err()
                .contains("重複")
        );
        let strings = crate::to_wasm_library(&with_unused, &["unused".into()]).unwrap();
        assert!(node(&strings, "new WebAssembly.Module(require('fs').readFileSync(0))").status.success());
        assert!(
            crate::to_wasm_library(&format!("{used}fn bad() -> int {{ true }}"), &names).is_err()
        );
        assert!(crate::to_wasm(used).unwrap_err().contains("main"));
    }

    #[test]
    fn runtime_reports_errors() {
        let script = r#"
let x; x = new WebAssembly.Instance(new WebAssembly.Module(require('fs').readFileSync(0)),
 { hibana: { write() {}, now_ms: () => 0n } }).exports;
try { x.main(); throw Error('trap expected'); } catch (e) { if (!(e instanceof WebAssembly.RuntimeError)) throw e; }
const v = new DataView(x.memory.buffer), p = Number(v.getBigUint64(24, true)), n = Number(v.getBigUint64(p, true));
process.stdout.write(Buffer.from(x.memory.buffer, p + 8, n));
"#;
        for (src, expected) in [
            ("fn f(a: int, b: int) -> int { a / b } fn main() { put(f(1, 0)) }", "0 で除算した"),
            ("fn f(a: int, b: int) -> int { a + b } fn main() { put(f(9007199254740991, 1)) }", "オーバーフロー"),
            ("fn f(a: int, b: int) -> int { a ** b } fn main() { put(f(2, -1)) }", "負の指数"),
            ("fn f(a: int, b: int) -> int { a ** b } fn main() { put(f(2, 63)) }", "オーバーフロー"),
        ] {
            let result = node(&crate::to_wasm(src).unwrap(), script);
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
            assert!(String::from_utf8_lossy(&result.stdout).contains(expected));
        }
    }

    #[test]
    fn executes_and_checks_integer_errors() {
        let src = "fn add(a: int, b: int) -> int { a + b }\nfn sub(a: int, b: int) -> int { a - b }\nfn mul(a: int, b: int) -> int { a * b }\nfn div(a: int, b: int) -> int { a / b }\nfn rem(a: int, b: int) -> int { a % b }\nfn main() { put(add(2, 3))\n put(mul(7, 6))\n put(sub(1, 2))\n put(div(7, 2))\n put(rem(7, 2)) }";
        let bytes = crate::to_wasm(src).unwrap();
        let mut expected = Vec::new();
        crate::run(src, &mut expected).unwrap();
        let script = r#"
const fs = require('fs');
const bytes = fs.readFileSync(0);
const m = new WebAssembly.Module(bytes);
let x;
x = new WebAssembly.Instance(m, {hibana: {write: (fd, p, n) => process.stdout.write(Buffer.from(x.memory.buffer, Number(p), Number(n)))}}).exports;
x.main();
function eq(value, expected) { if (value !== expected) throw Error(`${value} != ${expected}`); }
function trap(f, a, b) { try { f(a, b); } catch (e) { if (e instanceof WebAssembly.RuntimeError) return; throw e; } throw Error('trap expected'); }
const min = -9007199254740991, max = 9007199254740991;
eq(x.add(-4, 5), 1); eq(x.sub(-4, 5), -9); eq(x.mul(-4, 5), -20);
eq(x.div(-7, 3), -2); eq(x.rem(-7, 3), -1); eq(x.mul(min, 1), min);
trap(x.add, max, 1); trap(x.add, min, -1); trap(x.sub, min, 1); trap(x.sub, max, -1);
trap(x.mul, max, 2); eq(x.mul(min, -1), max); eq(x.mul(-1, min), max);
trap(x.div, 1, 0); eq(x.div(min, -1), max); trap(x.rem, 1, 0); eq(x.rem(min, -1), 0);
"#;
        let mut child = Command::new("node")
            .args(["-e", script])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected);
    }

    #[test]
    fn rejects_unsupported_programs() {
        assert!(
            crate::to_wasm(
                "extern 'browser' {\n    fn show(xs: Vec<int>)\n}\nfn main() { show([1]) }"
            )
            .unwrap_err()
            .contains("未対応")
        );
        assert!(crate::to_wasm("fn main() { let x = 1 + true }").is_err());
    }
}
