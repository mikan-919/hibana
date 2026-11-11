//! マクロが登録するビルド成果物。登録中にはファイルを読み書きしない。
use std::cell::RefCell;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub name: String,
    pub contents: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Text { path: String, contents: String },
    Wasm { path: String, exports: Vec<String> },
    Glue { path: String, exports: Vec<String> },
}

thread_local! {
    static REQUESTS: RefCell<Option<Vec<Request>>> = const { RefCell::new(None) };
}

struct Scope(Option<Vec<Request>>);
impl Drop for Scope {
    fn drop(&mut self) {
        REQUESTS.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

/// マクロ評価中の要求を集める。入れ子とパニックでも外側の状態を復元する。
pub fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<Request>) {
    let previous = REQUESTS.with(|slot| slot.replace(Some(Vec::new())));
    let scope = Scope(previous);
    let value = f();
    let requests = REQUESTS.with(|slot| slot.borrow_mut().take().unwrap());
    drop(scope);
    (value, requests)
}

pub fn register(request: Request) -> Result<(), String> {
    REQUESTS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let requests = slot
            .as_mut()
            .ok_or("ビルド成果物の登録には hibana build を使ってください")?;
        requests.push(request);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(path: &str) -> Request {
        Request::Text {
            path: path.into(),
            contents: "hello".into(),
        }
    }

    #[test]
    fn macro_registers_without_runtime_declarations() {
        let source = r#"
macro assets ("assets") -> Decl {
    build_emit("hello.txt", "hello");
    build_wasm("app.wasm", ["greet"]);
    build_glue("app.mjs", ["greet"]);
    quote {}
}
assets
"#;
        let (program, requests) = capture(|| crate::parser::parse(source));
        assert!(program.unwrap().fns.is_empty());
        assert_eq!(
            requests,
            vec![
                text("hello.txt"),
                Request::Wasm {
                    path: "app.wasm".into(),
                    exports: vec!["greet".into()]
                },
                Request::Glue {
                    path: "app.mjs".into(),
                    exports: vec!["greet".into()]
                },
            ]
        );
        assert!(crate::parser::parse(source).is_err());
    }

    #[test]
    fn rejects_build_calls_outside_macros_and_wrong_types() {
        for source in [
            r#"fn main() { build_emit("x", "y") }"#,
            r#"fn main() { build_wasm("x", ["f"]) }"#,
            r#"fn main() { build_glue("x", ["f"]) }"#,
        ] {
            assert!(crate::to_js(source).is_err());
        }
        for body in [
            r#"build_emit("x", 1)"#,
            r#"build_wasm("x", [1])"#,
            r#"build_glue(1, [])"#,
        ] {
            let source = format!("macro asset (\"asset\") -> Decl {{ {body}; quote {{}} }} asset");
            let (result, requests) = capture(|| crate::parser::parse(&source));
            assert!(result.is_err());
            assert!(requests.is_empty());
        }
    }

    #[test]
    fn capture_is_scoped() {
        assert!(register(text("outside")).is_err());
        let (value, requests) = capture(|| {
            register(text("before")).unwrap();
            let (_, inner) = capture(|| register(text("inner")).unwrap());
            assert_eq!(inner, vec![text("inner")]);
            register(text("after")).unwrap();
            42
        });
        assert_eq!(value, 42);
        assert_eq!(requests, vec![text("before"), text("after")]);
        assert!(register(text("outside")).is_err());
        assert!(capture(|| ()).1.is_empty());
    }

    #[test]
    fn panic_restores_capture() {
        let (_, requests) = capture(|| {
            register(text("before")).unwrap();
            let result = std::panic::catch_unwind(|| {
                capture(|| {
                    register(text("discarded")).unwrap();
                    panic!("test");
                })
            });
            assert!(result.is_err());
            register(text("after")).unwrap();
        });
        assert_eq!(requests, vec![text("before"), text("after")]);
        assert!(register(text("outside")).is_err());
    }
}
