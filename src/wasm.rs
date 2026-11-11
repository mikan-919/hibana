// 整数の IR を WebAssembly に変換する。
use crate::ir::{Instruction as Ir, Program};
use std::collections::HashMap;
use std::io::Write;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection,
    Function, FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction as I,
    MemorySection, MemoryType, Module, TypeSection, ValType,
};

// WASM の取り込みに整数の出力を渡して main を呼ぶ。
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

pub fn emit_selected(program: &Program, selected: Option<&[String]>) -> Vec<u8> {
    let has_print = program
        .functions
        .iter()
        .any(|f| f.instructions.iter().any(|i| matches!(i, Ir::Print(_))));
    let offset = u32::from(has_print) + program.imports.len() as u32;
    let mut indices: HashMap<&str, u32> = HashMap::new();
    let mut results = HashMap::new();
    for (i, f) in program.imports.iter().enumerate() {
        indices.insert(&f.name, u32::from(has_print) + i as u32);
        results.insert(f.name.as_str(), f.result_type);
    }
    for (i, f) in program.functions.iter().enumerate() {
        indices.insert(&f.name, offset + i as u32);
        results.insert(f.name.as_str(), f.result_type);
    }
    let strings = program.functions.iter().any(|f| {
        f.param_types.contains(&crate::ir::ValueType::Str)
            || f.result_type == crate::ir::ValueType::Str
            || f.instructions.iter().any(|i| matches!(i, Ir::Str(_)))
    }) || program.imports.iter().any(|f| {
        f.param_types.contains(&crate::ir::ValueType::Str)
            || f.result_type == crate::ir::ValueType::Str
    });
    let mut literals = HashMap::new();
    let mut data = DataSection::new();
    let mut heap = 8u32;
    for f in &program.functions {
        for inst in &f.instructions {
            if let Ir::Str(s) = inst {
                if !literals.contains_key(s) {
                    literals.insert(s.clone(), ((s.len() as u64) << 32 | heap as u64) as i64);
                    data.active(0, &ConstExpr::i32_const(heap as i32), s.bytes());
                    heap = heap
                        .checked_add(s.len() as u32)
                        .expect("文字列領域が大きすぎる");
                }
            }
        }
    }
    let mut types = TypeSection::new();
    if has_print {
        types.ty().function([ValType::I64], []);
    }
    for f in &program.imports {
        types.ty().function(
            vec![ValType::I64; f.param_types.len()],
            if f.result_type != crate::ir::ValueType::Unit {
                vec![ValType::I64]
            } else {
                vec![]
            },
        );
    }
    for f in &program.functions {
        types.ty().function(
            vec![ValType::I64; f.params],
            if f.result.is_some() {
                vec![ValType::I64]
            } else {
                vec![]
            },
        );
    }
    let helper_type = offset + program.functions.len() as u32;
    if strings {
        types.ty().function([ValType::I32], [ValType::I32]);
        types.ty().function([], []);
    }
    let mut imports = ImportSection::new();
    if has_print {
        imports.import("hibana", "print", EntityType::Function(0));
    }
    for (i, f) in program.imports.iter().enumerate() {
        imports.import(
            "hibana_host",
            &f.name,
            EntityType::Function(u32::from(has_print) + i as u32),
        );
    }
    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    for (index, f) in program.functions.iter().enumerate() {
        functions.function(index as u32 + offset);
        if selected.is_none_or(|names| names.contains(&f.name)) {
            exports.export(&f.name, ExportKind::Func, index as u32 + offset);
        }
        let mut body = Function::new([(f.instructions.len() as u32, ValType::I64)]);
        for (i, inst) in f.instructions.iter().enumerate() {
            let result = (f.params + i) as u32;
            match inst {
                Ir::Int(n) => {
                    body.instruction(&I::I64Const(*n));
                    body.instruction(&I::LocalSet(result));
                }
                Ir::Str(s) => {
                    body.instruction(&I::I64Const(literals[s]));
                    body.instruction(&I::LocalSet(result));
                }
                Ir::Call(name, args) => {
                    for arg in args {
                        body.instruction(&I::LocalGet(*arg as u32));
                    }
                    body.instruction(&I::Call(indices[name.as_str()]));
                    if results[name.as_str()] == crate::ir::ValueType::Unit {
                        body.instruction(&I::I64Const(0));
                    }
                    body.instruction(&I::LocalSet(result));
                }
                Ir::Print(value) => {
                    body.instruction(&I::LocalGet(*value as u32));
                    body.instruction(&I::Call(0));
                }
                Ir::Binary(op, a, b) => binary(&mut body, op, *a as u32, *b as u32, result),
            }
        }
        if let Some(value) = f.result {
            body.instruction(&I::LocalGet(value as u32));
        }
        body.instruction(&I::End);
        code.function(&body);
    }
    let mut memory = MemorySection::new();
    let mut globals = GlobalSection::new();
    if strings {
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
        exports.export("__hibana_alloc", ExportKind::Func, helper_type);
        exports.export("__hibana_reset", ExportKind::Func, helper_type + 1);
        functions.function(helper_type);
        functions.function(helper_type + 1);
        // ponytail: 呼び出し単位の領域。返却後に reset できる値だけを扱い、永続オブジェクトは未対応。
        let mut alloc = Function::new([(2, ValType::I32)]);
        alloc.instruction(&I::GlobalGet(0));
        alloc.instruction(&I::LocalTee(1));
        alloc.instruction(&I::LocalGet(0));
        alloc.instruction(&I::I32Add);
        alloc.instruction(&I::LocalTee(2));
        alloc.instruction(&I::LocalGet(1));
        alloc.instruction(&I::I32LtU);
        trap_if(&mut alloc);
        // 必要なページ数は i64 で計算し、32 ビットの加算の周回を避ける。
        alloc.instruction(&I::LocalGet(2));
        alloc.instruction(&I::I64ExtendI32U);
        alloc.instruction(&I::I64Const(65535));
        alloc.instruction(&I::I64Add);
        alloc.instruction(&I::I64Const(16));
        alloc.instruction(&I::I64ShrU);
        alloc.instruction(&I::I32WrapI64);
        alloc.instruction(&I::MemorySize(0));
        alloc.instruction(&I::I32GtU);
        alloc.instruction(&I::If(BlockType::Empty));
        alloc.instruction(&I::LocalGet(2));
        alloc.instruction(&I::I64ExtendI32U);
        alloc.instruction(&I::I64Const(65535));
        alloc.instruction(&I::I64Add);
        alloc.instruction(&I::I64Const(16));
        alloc.instruction(&I::I64ShrU);
        alloc.instruction(&I::I32WrapI64);
        alloc.instruction(&I::MemorySize(0));
        alloc.instruction(&I::I32Sub);
        alloc.instruction(&I::MemoryGrow(0));
        alloc.instruction(&I::I32Const(-1));
        alloc.instruction(&I::I32Eq);
        trap_if(&mut alloc);
        alloc.instruction(&I::End);
        alloc.instruction(&I::LocalGet(2));
        alloc.instruction(&I::GlobalSet(0));
        alloc.instruction(&I::LocalGet(1));
        alloc.instruction(&I::End);
        code.function(&alloc);
        let mut reset = Function::new([]);
        reset.instruction(&I::I32Const(heap as i32));
        reset.instruction(&I::GlobalSet(0));
        reset.instruction(&I::End);
        code.function(&reset);
    }
    let mut module = Module::new();
    module.section(&types);
    if has_print || !program.imports.is_empty() {
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

// WASM の整数演算は周回するため、hibana の範囲外エラーを検査する。
fn binary(f: &mut Function, op: &str, a: u32, b: u32, result: u32) {
    if op == "%" {
        f.instruction(&I::LocalGet(a));
        f.instruction(&I::I64Const(i64::MIN));
        f.instruction(&I::I64Eq);
        f.instruction(&I::LocalGet(b));
        f.instruction(&I::I64Const(-1));
        f.instruction(&I::I64Eq);
        f.instruction(&I::I32And);
        trap_if(f);
    }
    f.instruction(&I::LocalGet(a));
    f.instruction(&I::LocalGet(b));
    f.instruction(&match op {
        "+" => I::I64Add,
        "-" => I::I64Sub,
        "*" => I::I64Mul,
        "/" => I::I64DivS,
        "%" => I::I64RemS,
        _ => unreachable!(),
    });
    f.instruction(&I::LocalSet(result));
    match op {
        "+" | "-" => {
            f.instruction(&I::LocalGet(a));
            f.instruction(&I::LocalGet(if op == "+" { result } else { b }));
            f.instruction(&I::I64Xor);
            f.instruction(&I::LocalGet(if op == "+" { b } else { a }));
            f.instruction(&I::LocalGet(result));
            f.instruction(&I::I64Xor);
            f.instruction(&I::I64And);
            f.instruction(&I::I64Const(0));
            f.instruction(&I::I64LtS);
            trap_if(f);
        }
        "*" => {
            f.instruction(&I::LocalGet(a));
            f.instruction(&I::I64Eqz);
            f.instruction(&I::I32Eqz);
            f.instruction(&I::If(BlockType::Empty));
            f.instruction(&I::LocalGet(result));
            f.instruction(&I::LocalGet(a));
            f.instruction(&I::I64DivS);
            f.instruction(&I::LocalGet(b));
            f.instruction(&I::I64Ne);
            trap_if(f);
            f.instruction(&I::End);
        }
        _ => {}
    }
}

fn trap_if(f: &mut Function) {
    f.instruction(&I::If(BlockType::Empty));
    f.instruction(&I::Unreachable);
    f.instruction(&I::End);
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[test]
    fn string_memory_grows_resets_and_preserves_literals() {
        let bytes =
            crate::to_wasm_library("fn literal() -> str { \"日本語\" }", &["literal".into()])
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
            "{used}fn unused() -> str {{ \"unused\" }}\nfn unused_print() {{ print(999) }}\n"
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

        assert!(crate::to_wasm_library(used, &["missing".into()])
            .unwrap_err()
            .contains("無い"));
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
        assert!(super::run(&crate::to_wasm(src).unwrap(), Fails)
            .err()
            .unwrap()
            .contains("出力失敗"));
    }

    #[test]
    fn executes_and_checks_integer_errors() {
        let src = "fn add(a: int, b: int) -> int { a + b }\nfn sub(a: int, b: int) -> int { a - b }\nfn mul(a: int, b: int) -> int { a * b }\nfn div(a: int, b: int) -> int { a / b }\nfn rem(a: int, b: int) -> int { a % b }\nfn main() { print(add(2, 3))\n print(mul(7, 6)) }";
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
        assert!(crate::to_wasm("fn main() { print(\"hello\") }")
            .unwrap_err()
            .contains("未対応"));
        assert!(crate::to_wasm("fn main() { let x = 1 + true }").is_err());
    }
}
