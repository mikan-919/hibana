// 構造化 IR を WebAssembly に変換する。
use crate::ir::{Block, Expr, Node, Op, Program, Stmt, ValueType};
use std::collections::HashMap;
use std::io::Write;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection,
    Function, FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction as I,
    MemorySection, MemoryType, Module, TypeSection, ValType,
};

// WASM の取り込みに出力を渡して main を呼ぶ。
pub fn run<W: Write + 'static>(bytes: &[u8], out: W) -> crate::error::R<W> {
    let engine = wasmtime::Engine::default();
    let module = wasmtime::Module::new(&engine, bytes)
        .map_err(|e| format!("WASM の読み込みに失敗した: {e}"))?;
    let mut store = wasmtime::Store::new(&engine, out);
    let mut linker = wasmtime::Linker::new(&engine);
    linker
        .func_wrap(
            "hibana",
            "print",
            |mut caller: wasmtime::Caller<'_, W>, value: i64| -> wasmtime::Result<()> {
                writeln!(caller.data_mut(), "{value}")?;
                Ok(())
            },
        )
        .and_then(|l| {
            l.func_wrap(
                "hibana",
                "print_str",
                |mut caller: wasmtime::Caller<'_, W>, packed: i64| -> wasmtime::Result<()> {
                    let memory = caller
                        .get_export("memory")
                        .and_then(|e| e.into_memory())
                        .ok_or_else(|| wasmtime::Error::msg("WASM に memory が無い"))?;
                    let (ptr, len) = (
                        packed as u64 as u32 as usize,
                        (packed as u64 >> 32) as usize,
                    );
                    let bytes = memory
                        .data(&caller)
                        .get(ptr..ptr + len)
                        .ok_or_else(|| wasmtime::Error::msg("文字列が memory の範囲外"))?
                        .to_vec();
                    caller.data_mut().write_all(&bytes)?;
                    writeln!(caller.data_mut())?;
                    Ok(())
                },
            )
        })
        .map_err(|e| format!("WASM の出力関数を登録できない: {e}"))?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|e| format!("WASM の初期化に失敗した: {e}"))?;
    let main = instance
        .get_typed_func::<(), ()>(&mut store, "main")
        .map_err(|e| format!("WASM の main を呼べない: {e}"))?;
    main.call(&mut store, ()).map_err(|e| {
        if let Some(error) = e.downcast_ref::<std::io::Error>() {
            return crate::error::Error::from(format!("出力に失敗した: {error}"));
        }
        let msg = match e.downcast_ref::<wasmtime::Trap>() {
            Some(wasmtime::Trap::IntegerDivisionByZero) => "0 で除算した".to_string(),
            Some(wasmtime::Trap::IntegerOverflow | wasmtime::Trap::UnreachableCodeReached) => {
                "オーバーフロー".to_string()
            }
            _ => format!("WASM の実行に失敗した: {e}"),
        };
        crate::error::Error::from(msg)
    })?;
    Ok(store.into_data())
}

pub fn emit(program: &Program) -> Vec<u8> {
    emit_selected(program, None)
}

// 関数の番号と型の番号は同じ並びにする（取り込み、定義した関数、補助関数の順）
pub fn emit_selected(program: &Program, selected: Option<&[String]>) -> Vec<u8> {
    let mut print_int = false;
    let mut print_str = false;
    let mut literals: HashMap<String, i64> = HashMap::new();
    let mut data = DataSection::new();
    let mut heap = 8u32;
    for f in &program.functions {
        f.each(&mut |n| match n {
            Node::Stmt(Stmt::Print(ValueType::Int, _)) => print_int = true,
            Node::Stmt(Stmt::Print(..)) => print_str = true,
            Node::Expr(Expr::Str(s)) => {
                if !literals.contains_key(s) {
                    literals.insert(s.clone(), ((s.len() as u64) << 32 | heap as u64) as i64);
                    data.active(0, &ConstExpr::i32_const(heap as i32), s.bytes());
                    heap = heap
                        .checked_add(s.len() as u32)
                        .expect("文字列領域が大きすぎる");
                }
            }
            _ => {}
        });
    }
    let has_str =
        |ts: &[ValueType], r: ValueType| ts.contains(&ValueType::Str) || r == ValueType::Str;
    let strings = !literals.is_empty()
        || program
            .functions
            .iter()
            .any(|f| has_str(&f.param_types, f.result_type))
        || program
            .imports
            .iter()
            .any(|f| has_str(&f.param_types, f.result_type));
    let results = |r: ValueType| {
        if r == ValueType::Unit {
            vec![]
        } else {
            vec![ValType::I64]
        }
    };
    let mut types = TypeSection::new();
    let mut imports = ImportSection::new();
    let mut indices: HashMap<&str, u32> = HashMap::new();
    let mut units: HashMap<&str, bool> = HashMap::new();
    let mut count = 0u32;
    let mut import = |module: &str, name: &str, params: usize, result: ValueType| {
        types
            .ty()
            .function(vec![ValType::I64; params], results(result));
        imports.import(module, name, EntityType::Function(count));
        count += 1;
        count - 1
    };
    let print_int = print_int.then(|| import("hibana", "print", 1, ValueType::Unit));
    let print_str = print_str.then(|| import("hibana", "print_str", 1, ValueType::Unit));
    for f in &program.imports {
        let i = import("hibana_host", &f.name, f.param_types.len(), f.result_type);
        indices.insert(&f.name, i);
        units.insert(&f.name, f.result_type == ValueType::Unit);
    }
    let imported = count;
    for f in &program.functions {
        types.ty().function(
            vec![ValType::I64; f.param_types.len()],
            results(f.result_type),
        );
        indices.insert(&f.name, count);
        units.insert(&f.name, f.result_type == ValueType::Unit);
        count += 1;
    }
    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    for (i, f) in program.functions.iter().enumerate() {
        let index = imported + i as u32;
        functions.function(index);
        if selected.is_none_or(|names| names.contains(&f.name)) {
            exports.export(&f.name, ExportKind::Func, index);
        }
        let mut e = Emit {
            out: Vec::new(),
            locals: f.locals,
            indices: &indices,
            units: &units,
            literals: &literals,
            print_int,
            print_str,
        };
        e.stmts(&f.body);
        if f.result_type != ValueType::Unit {
            e.out.push(I::Unreachable);
        }
        e.out.push(I::End);
        let params = f.param_types.len() as u32;
        code.function(&function(&[(e.locals - params, ValType::I64)], &e.out));
    }
    let mut memory = MemorySection::new();
    let mut globals = GlobalSection::new();
    if strings {
        types.ty().function([ValType::I32], [ValType::I32]);
        types.ty().function([], []);
        memory.memory(MemoryType {
            minimum: (heap as u64).div_ceil(65536).max(1),
            maximum: Some(65536),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(heap as i32),
        );
        exports.export("memory", ExportKind::Memory, 0);
        exports.export("__hibana_alloc", ExportKind::Func, count);
        exports.export("__hibana_reset", ExportKind::Func, count + 1);
        functions.function(count);
        functions.function(count + 1);
        // ponytail: 呼び出し単位の領域。返却後に reset できる値だけを扱い、永続オブジェクトは未対応。
        // 必要なページ数は i64 で計算し、32 ビットの加算の周回を避ける。
        let pages = [
            I::LocalGet(2),
            I::I64ExtendI32U,
            I::I64Const(65535),
            I::I64Add,
            I::I64Const(16),
            I::I64ShrU,
            I::I32WrapI64,
            I::MemorySize(0),
        ];
        let mut alloc = vec![
            I::GlobalGet(0),
            I::LocalTee(1),
            I::LocalGet(0),
            I::I32Add,
            I::LocalTee(2),
            I::LocalGet(1),
            I::I32LtU,
        ];
        trap_if(&mut alloc);
        alloc.extend(pages.clone());
        alloc.extend([I::I32GtU, I::If(BlockType::Empty)]);
        alloc.extend(pages);
        alloc.extend([I::I32Sub, I::MemoryGrow(0), I::I32Const(-1), I::I32Eq]);
        trap_if(&mut alloc);
        alloc.extend([
            I::End,
            I::LocalGet(2),
            I::GlobalSet(0),
            I::LocalGet(1),
            I::End,
        ]);
        code.function(&function(&[(2, ValType::I32)], &alloc));
        let reset = [I::I32Const(heap as i32), I::GlobalSet(0), I::End];
        code.function(&function(&[], &reset));
    }
    let mut module = Module::new();
    module.section(&types);
    if imported > 0 {
        module.section(&imports);
    }
    module.section(&functions);
    if strings {
        module.section(&memory);
        module.section(&globals);
    }
    module.section(&exports);
    module.section(&code);
    if strings {
        module.section(&data);
    }
    module.finish()
}

fn function(locals: &[(u32, ValType)], body: &[I<'static>]) -> Function {
    let mut f = Function::new(locals.iter().copied());
    for i in body {
        f.instruction(i);
    }
    f
}

struct Emit<'a> {
    out: Vec<I<'static>>,
    // 引数・局所変数・演算用の一時変数を合わせた数
    locals: u32,
    indices: &'a HashMap<&'a str, u32>,
    units: &'a HashMap<&'a str, bool>,
    literals: &'a HashMap<String, i64>,
    print_int: Option<u32>,
    print_str: Option<u32>,
}

impl Emit<'_> {
    fn temp(&mut self) -> u32 {
        self.locals += 1;
        self.locals - 1
    }

    fn stmts(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            match s {
                Stmt::Set(i, e) => {
                    self.expr(e);
                    self.out.push(I::LocalSet(*i));
                }
                Stmt::Eval(e) => {
                    self.expr(e);
                    self.out.push(I::Drop);
                }
                Stmt::Print(t, e) => {
                    self.expr(e);
                    let f = if *t == ValueType::Int {
                        self.print_int
                    } else {
                        self.print_str
                    };
                    self.out.push(I::Call(f.unwrap()));
                }
                Stmt::While(c, body) => {
                    self.out.push(I::Block(BlockType::Empty));
                    self.out.push(I::Loop(BlockType::Empty));
                    self.expr(c);
                    self.out.extend([I::I64Eqz, I::BrIf(1)]);
                    self.stmts(body);
                    self.out.extend([I::Br(0), I::End, I::End]);
                }
                Stmt::Return(e) => {
                    if let Some(e) = e {
                        self.expr(e);
                    }
                    self.out.push(I::Return);
                }
            }
        }
    }

    fn block(&mut self, b: &Block) {
        self.stmts(&b.stmts);
        self.expr(&b.value);
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Int(n) => self.out.push(I::I64Const(*n)),
            Expr::Str(s) => self.out.push(I::I64Const(self.literals[s])),
            Expr::Local(i) => self.out.push(I::LocalGet(*i)),
            Expr::Not(a) => {
                self.expr(a);
                self.out.extend([I::I64Eqz, I::I64ExtendI32U]);
            }
            Expr::Call(name, args) => {
                for a in args {
                    self.expr(a);
                }
                self.out.push(I::Call(self.indices[name.as_str()]));
                if self.units[name.as_str()] {
                    self.out.push(I::I64Const(0));
                }
            }
            Expr::If(c, t, f) => {
                self.expr(c);
                self.out
                    .extend([I::I64Eqz, I::I32Eqz, I::If(BlockType::Result(ValType::I64))]);
                self.block(t);
                self.out.push(I::Else);
                self.block(f);
                self.out.push(I::End);
            }
            Expr::Block(b) => self.block(b),
            Expr::Binary(op, a, b) => {
                let cmp = match op {
                    Op::Eq => Some(I::I64Eq),
                    Op::Ne => Some(I::I64Ne),
                    Op::Lt => Some(I::I64LtS),
                    Op::Le => Some(I::I64LeS),
                    Op::Gt => Some(I::I64GtS),
                    Op::Ge => Some(I::I64GeS),
                    _ => None,
                };
                if let Some(cmp) = cmp {
                    self.expr(a);
                    self.expr(b);
                    self.out.extend([cmp, I::I64ExtendI32U]);
                    return;
                }
                let (ta, tb, tr) = (self.temp(), self.temp(), self.temp());
                self.expr(a);
                self.out.push(I::LocalSet(ta));
                self.expr(b);
                self.out.push(I::LocalSet(tb));
                binary(&mut self.out, *op, ta, tb, tr);
                self.out.push(I::LocalGet(tr));
            }
        }
    }
}

// WASM の整数演算は周回するため、hibana の範囲外エラーを検査する。
fn binary(f: &mut Vec<I<'static>>, op: Op, a: u32, b: u32, result: u32) {
    if op == Op::Rem {
        f.extend([
            I::LocalGet(a),
            I::I64Const(i64::MIN),
            I::I64Eq,
            I::LocalGet(b),
            I::I64Const(-1),
            I::I64Eq,
            I::I32And,
        ]);
        trap_if(f);
    }
    f.extend([I::LocalGet(a), I::LocalGet(b)]);
    f.push(match op {
        Op::Add => I::I64Add,
        Op::Sub => I::I64Sub,
        Op::Mul => I::I64Mul,
        Op::Div => I::I64DivS,
        Op::Rem => I::I64RemS,
        _ => unreachable!(),
    });
    f.push(I::LocalSet(result));
    match op {
        Op::Add | Op::Sub => {
            let add = op == Op::Add;
            f.extend([
                I::LocalGet(a),
                I::LocalGet(if add { result } else { b }),
                I::I64Xor,
                I::LocalGet(if add { b } else { a }),
                I::LocalGet(result),
                I::I64Xor,
                I::I64And,
                I::I64Const(0),
                I::I64LtS,
            ]);
            trap_if(f);
        }
        Op::Mul => {
            f.extend([
                I::LocalGet(a),
                I::I64Eqz,
                I::I32Eqz,
                I::If(BlockType::Empty),
                I::LocalGet(result),
                I::LocalGet(a),
                I::I64DivS,
                I::LocalGet(b),
                I::I64Ne,
            ]);
            trap_if(f);
            f.push(I::End);
        }
        _ => {}
    }
}

fn trap_if(f: &mut Vec<I<'static>>) {
    f.extend([I::If(BlockType::Empty), I::Unreachable, I::End]);
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn string_memory_grows_resets_and_preserves_literals() {
        let bytes = crate::to_wasm_library("fn literal() -> str { '日本語' }", &["literal".into()])
            .unwrap();
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let literal = instance
            .get_typed_func::<(), i64>(&mut store, "literal")
            .unwrap();
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "__hibana_alloc")
            .unwrap();
        let reset = instance
            .get_typed_func::<(), ()>(&mut store, "__hibana_reset")
            .unwrap();
        let memory = instance.get_memory(&mut store, "memory").unwrap();
        let packed = literal.call(&mut store, ()).unwrap() as u64;
        let ptr = packed as u32 as usize;
        let len = (packed >> 32) as usize;
        assert_eq!(&memory.data(&store)[ptr..ptr + len], "日本語".as_bytes());
        let first = alloc.call(&mut store, 200_000).unwrap();
        assert!(memory.data_size(&store) >= first as usize + 200_000);
        assert!(alloc.call(&mut store, -1).is_err());
        reset.call(&mut store, ()).unwrap();
        assert_eq!(alloc.call(&mut store, 1).unwrap(), first);
        assert_eq!(literal.call(&mut store, ()).unwrap() as u64, packed);
        assert_eq!(&memory.data(&store)[ptr..ptr + len], "日本語".as_bytes());
    }

    #[test]
    fn library_exports_only_requested_reachable_functions() {
        let used = "fn plus(a: int, b: int) -> int { a + b }\nfn twice(n: int) -> int { plus(n, n) }\nfn result(n: int) -> int { twice(n) + 1 }\n";
        let with_unused = format!(
            "{used}fn unused() -> str {{ 'unused' }}\nfn unused_print() {{ print(999) }}\n"
        );
        let names = vec!["result".to_string()];
        let bytes = crate::to_wasm_library(&with_unused, &names).unwrap();
        assert_eq!(bytes, crate::to_wasm_library(used, &names).unwrap());
        let script = r#"
const m = new WebAssembly.Module(require('fs').readFileSync(0));
if (JSON.stringify(WebAssembly.Module.imports(m)) !== '[]') throw Error('unexpected imports');
const x = new WebAssembly.Instance(m, {}).exports;
if (Object.keys(x).join(',') !== 'result' || x.result(20n) !== 41n) throw Error('unexpected exports or result');
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
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, bytes).unwrap();
        let names: Vec<_> = module
            .exports()
            .map(|export| export.name().to_string())
            .collect();
        assert_eq!(names, ["plus", "result"]);
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        assert_eq!(
            instance
                .get_typed_func::<(i64, i64), i64>(&mut store, "plus")
                .unwrap()
                .call(&mut store, (20, 22))
                .unwrap(),
            42
        );

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
        assert!(wasmtime::Module::new(&engine, strings).is_ok());
        assert!(
            crate::to_wasm_library(&format!("{used}fn bad() -> int {{ true }}"), &names).is_err()
        );
        assert!(crate::to_wasm(used).unwrap_err().contains("main"));
    }

    #[test]
    fn embedded_runtime_matches_interpreter_and_reports_errors() {
        let src = "fn add(a: int, b: int) -> int { a + b }\nfn main() { print(add(19, 23)) }";
        let mut expected = Vec::new();
        crate::run(src, &mut expected).unwrap();
        let output = super::run(&crate::to_wasm(src).unwrap(), Vec::new()).unwrap();
        assert_eq!(output, expected);

        let src = "fn div(a: int, b: int) -> int { a / b }\nfn main() { print(div(1, 0)) }";
        let err = super::run(&crate::to_wasm(src).unwrap(), Vec::new()).unwrap_err();
        assert!(err.contains("0 で除算した"), "{err}");

        let src = "fn add(a: int, b: int) -> int { a + b }\nfn main() { print(add(9223372036854775807, 1)) }";
        let err = super::run(&crate::to_wasm(src).unwrap(), Vec::new()).unwrap_err();
        assert!(err.contains("オーバーフロー"), "{err}");

        struct Fails;
        impl Write for Fails {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("出力失敗"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let src = "fn main() { print(1) }";
        assert!(
            super::run(&crate::to_wasm(src).unwrap(), Fails)
                .err()
                .unwrap()
                .contains("出力失敗")
        );
    }

    #[test]
    fn executes_and_checks_integer_errors() {
        let src = "fn add(a: int, b: int) -> int { a + b }\nfn sub(a: int, b: int) -> int { a - b }\nfn mul(a: int, b: int) -> int { a * b }\nfn div(a: int, b: int) -> int { a / b }\nfn rem(a: int, b: int) -> int { a % b }\nfn main() { print(add(2, 3))\n print(mul(7, 6))\n print(sub(1, 2))\n print(div(7, 2))\n print(rem(7, 2)) }";
        let bytes = crate::to_wasm(src).unwrap();
        let mut expected = Vec::new();
        crate::run(src, &mut expected).unwrap();
        let script = r#"
const fs = require('fs');
const bytes = fs.readFileSync(0);
const m = new WebAssembly.Module(bytes);
const x = new WebAssembly.Instance(m, {hibana: {print: n => console.log(n.toString())}}).exports;
x.main();
function eq(value, expected) { if (value !== expected) throw Error(`${value} != ${expected}`); }
function trap(f, a, b) { try { f(a, b); } catch (e) { if (e instanceof WebAssembly.RuntimeError) return; throw e; } throw Error('trap expected'); }
const min = -(1n << 63n), max = (1n << 63n) - 1n;
eq(x.add(-4n, 5n), 1n); eq(x.sub(-4n, 5n), -9n); eq(x.mul(-4n, 5n), -20n);
eq(x.div(-7n, 3n), -2n); eq(x.rem(-7n, 3n), -1n); eq(x.mul(min, 1n), min);
trap(x.add, max, 1n); trap(x.add, min, -1n); trap(x.sub, min, 1n); trap(x.sub, max, -1n);
trap(x.mul, max, 2n); trap(x.mul, min, -1n); trap(x.mul, -1n, min);
trap(x.div, 1n, 0n); trap(x.div, min, -1n); trap(x.rem, 1n, 0n); trap(x.rem, min, -1n);
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
            crate::to_wasm("fn main() { let x = [1] }")
                .unwrap_err()
                .contains("未対応")
        );
        assert!(crate::to_wasm("fn main() { let x = 1 + true }").is_err());
    }
}
