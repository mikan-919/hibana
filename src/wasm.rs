// 整数の IR を WebAssembly に変換する。
use crate::ir::{Instruction as Ir, Program};
use std::collections::HashMap;
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    ImportSection, Instruction as I, Module, TypeSection, ValType,
};

pub fn emit(program: &Program) -> Vec<u8> {
    let has_print = program
        .functions
        .iter()
        .any(|f| f.instructions.iter().any(|i| matches!(i, Ir::Print(_))));
    let offset = u32::from(has_print);
    let indices: HashMap<_, _> = program
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), i as u32 + offset))
        .collect();
    let mut types = TypeSection::new();
    if has_print {
        types.ty().function([ValType::I64], []);
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
    let mut imports = ImportSection::new();
    if has_print {
        imports.import("hibana", "print", EntityType::Function(0));
    }
    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    for (index, f) in program.functions.iter().enumerate() {
        functions.function(index as u32 + offset);
        exports.export(&f.name, ExportKind::Func, index as u32 + offset);
        let mut body = Function::new([(f.instructions.len() as u32, ValType::I64)]);
        for (i, inst) in f.instructions.iter().enumerate() {
            let result = (f.params + i) as u32;
            match inst {
                Ir::Int(n) => {
                    body.instruction(&I::I64Const(*n));
                    body.instruction(&I::LocalSet(result));
                }
                Ir::Call(name, args) => {
                    for arg in args {
                        body.instruction(&I::LocalGet(*arg as u32));
                    }
                    body.instruction(&I::Call(indices[name.as_str()]));
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
    let mut module = Module::new();
    module.section(&types);
    if has_print {
        module.section(&imports);
    }
    module.section(&functions);
    module.section(&exports);
    module.section(&code);
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
        assert!(
            crate::to_wasm("fn main() { print(\"hello\") }")
                .unwrap_err()
                .contains("未対応")
        );
        assert!(crate::to_wasm("fn main() { let x = 1 + true }").is_err());
    }
}
