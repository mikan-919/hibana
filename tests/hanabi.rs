use hibana::pipeline::{self, Kind};
use std::path::Path;

fn check(source: &str) -> hibana::error::R<pipeline::Checked> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("hanabi");
    let expanded = pipeline::load(source, &root)?;
    pipeline::check(pipeline::parse(&expanded)?, Kind::Executable)
}

#[test]
fn framework_sources_do_not_embed_demo_contracts_or_handlers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("hanabi");
    for entry in std::fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "hb") || path.ends_with("browser.mjs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for forbidden in [
                "#post-form",
                "#posts",
                "#notice",
                "#app",
                "/api/posts",
                "data/posts.json",
                "8787",
                "demo/",
                "fn submit(",
                "fn edit(",
                "fn cancel(",
                "data-id",
                "state('pending'",
                "new Map(",
            ] {
                assert!(!text.contains(forbidden), "{}: {forbidden}", path.display());
            }
        }
    }
}

#[test]
fn contracts_use_generated_codecs_typed_commands_and_replaceable_storage() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("hanabi/tests");
    let source = std::fs::read_to_string(root.join("contracts.hb")).unwrap();
    let expanded = pipeline::load(&source, &root).unwrap();
    let checked = pipeline::check(pipeline::parse(&expanded).unwrap(), Kind::Executable).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(checked.program(), &mut output).unwrap();
    assert_eq!(output, b"hanabi contracts passed\n");
    let ir = pipeline::lower(&checked, Some(&["main".into()])).unwrap();
    assert!(pipeline::emit_wasm(&ir, Some(&["main".into()])).is_ok());
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    assert!(hibana::native::emit(&ir).is_ok());
}

#[test]
fn invalid_static_boundaries_and_contract_connections_are_rejected() {
    let cases = [
        ("use './types.hb' -> *\nfn main() { let p: Port = 0 }", "証明できない"),
        ("use './server.hb'\nfn main() { server.text(99, 'bad') }", "証明できない"),
        ("use './types.hb' -> *\nfn main() { let b: Byte = 256 }", "証明できない"),
        ("use './native.hb'\nfn main() { native.slice([1, 2], 0, 3) }", "証明できない"),
        ("use './native.hb'\nfn main() { native.slice([1, 2], -1, 1) }", "証明できない"),
        ("use './demo/domain.hb' -> *\nfn main() { Title('') }", "証明できない"),
        ("use './demo/domain.hb' -> *\nfn main() { PostId(0) }", "証明できない"),
        ("use './demo/stores.hb'\nfn main() { stores.list(stores.Repository { path: 'test.json' }) }", "with storage.Storage が必要"),
        ("use './demo/api.hb' -> { update_api }\nuse './demo/domain.hb' -> { PostId }\nuse './client.hb'\nfn main() { client.call(update_api(), PostId(1), 42, fn(v) { v }) }", "domain.Form"),
        ("use './routes.hb' -> { route }\nuse './demo/api.hb' -> { update_api }\nuse './server.hb' -> { Request }\nfn handler(request: Request) { route(request, update_api(), fn(_, _) { Ok(42) }) }\nfn main() {}", "domain.Post"),
        ("use './client.hb'\nuse './endpoint.hb' -> { Request }\nuse './types.hb' -> { Get }\nfn main() { client.exchange(Request { method: Get, url: '/', body: 'null' }) }", "効果"),
        ("use './endpoint.hb' -> *\napi { first GET '/x/{a}' (int, unit) -> int ! Fault = 200 second GET '/x/{b}' (int, unit) -> int ! Fault = 200 }\nfn main() {}", "経路が重複"),
        ("use './view.hb'\nfn main() { <div></span> }", "閉じタグが一致しない"),
        ("use './view.hb'\nfn main() { <button onClick=\"edit\">編集</button> }", "イベントには型付き"),
        ("use './view.hb'\nfn main() { <div>{42}</div> }", "実装が無い"),
    ];
    for (source, message) in cases {
        let error = match check(source) {
            Ok(_) => panic!("拒否しなかった: {source}"),
            Err(e) => e.to_string(),
        };
        assert!(error.contains(message), "{source}\n{error}");
    }
}

#[test]
fn native_host_signatures_accept_refinements_and_reject_wasm_io() {
    check("use './native.hb'\nfn main() { let bytes = native.slice([1, 2], 0, 1)\n let n: int[x -> x == 1] = len(bytes)\n put(bytes[0]) }").unwrap();
    check("use './native.hb'\nfn main() { let bytes = native.join([1], [2])\n let n: int[x -> x == 2] = len(bytes)\n put(bytes[1]) }").unwrap();
    let checked = check("use './native.hb'\nfn main() { native.listen(12345) }").unwrap();
    let ir = pipeline::lower(&checked, Some(&["main".into()])).unwrap();
    assert!(pipeline::emit_wasm(&ir, Some(&["main".into()]))
        .unwrap_err()
        .to_string()
        .contains("ネイティブ"));
    for source in [
        "extern 'hibana:native' { fn listen(port: String) -> int }\nfn main() { put(listen('x')) }",
        "extern 'hibana:native' { fn unknown() -> int }\nfn main() { put(unknown()) }",
    ] {
        let checked = check(source).unwrap();
        assert!(pipeline::lower(&checked, Some(&["main".into()]))
            .unwrap_err()
            .to_string()
            .contains("hibana:native"));
    }
}

#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
mod native {
    use serde_json::{json, Value};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    struct Directory(PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    struct Server(Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn start(dir: &Path, port: u16) -> Server {
        let mut server = Server(
            Command::new(dir.join("application"))
                .current_dir(dir)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        for _ in 0..100 {
            if server.0.try_wait().unwrap().is_some() {
                panic!("サーバーが起動前に終了した");
            }
            if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .unwrap();
                let mut bytes = Vec::new();
                stream.read_to_end(&mut bytes).unwrap();
                assert!(bytes.starts_with(b"HTTP/1.1 200"));
                return server;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("サーバーが起動しない");
    }
    fn exchange(port: u16, bytes: &[u8]) -> (u16, Vec<u8>) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(bytes).unwrap();
        let mut output = Vec::new();
        stream.read_to_end(&mut output).unwrap();
        let end = output
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("HTTP 応答が無い")
            + 4;
        let status = std::str::from_utf8(&output[..end])
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (status, output[end..].to_vec())
    }
    fn request(port: u16, method: &str, path: &str, value: Option<Value>) -> (u16, Vec<u8>) {
        let body = value
            .map(|v| serde_json::to_vec(&v).unwrap())
            .unwrap_or_default();
        let mut bytes = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nX-Hanabi-Request: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        bytes.extend(body);
        exchange(port, &bytes)
    }
    #[test]
    fn build_script_http_validation_assets_and_restart_persistence() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let output = root.join("target/hanabi");
        let build = Command::new(env!("CARGO_BIN_EXE_hibana"))
            .current_dir(root)
            .arg("hanabi/demo/build.hb")
            .arg("--")
            .arg(root)
            .arg(&output)
            .arg(port.to_string())
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}{}",
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        let dir =
            Directory(std::env::temp_dir().join(format!("hanabi-http-{}", std::process::id())));
        std::fs::create_dir_all(&dir.0).unwrap();
        for name in [
            "application",
            "client.wasm",
            "client.mjs",
            "browser.mjs",
            "main.mjs",
            "index.html",
            "style.css",
        ] {
            assert!(output.join(name).is_file(), "生成物が無い: {name}");
            std::fs::copy(output.join(name), dir.0.join(name)).unwrap();
        }
        assert!(std::fs::read(dir.0.join("client.wasm"))
            .unwrap()
            .starts_with(b"\0asm"));
        std::fs::set_permissions(
            dir.0.join("application"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        drop(listener);
        let server = start(&dir.0, port);
        assert_eq!(
            request(port, "GET", "/api/posts", None),
            (200, b"[]".to_vec())
        );
        let (status, body) = request(
            port,
            "POST",
            "/api/posts",
            Some(json!({"title":"<script>題</script>","body":"本文🌸"})),
        );
        assert_eq!(status, 201, "{}", String::from_utf8_lossy(&body));
        let post: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(post["id"], 1);
        let (status, body) = request(
            port,
            "PUT",
            "/api/posts/1",
            Some(json!({"title":"編集後","body":"更新した本文"})),
        );
        assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
        for value in [
            json!({"title":"","body":"本文"}),
            json!({"title":"x".repeat(121),"body":"本文"}),
            json!({"title":42,"body":"本文"}),
        ] {
            assert_eq!(request(port, "POST", "/api/posts", Some(value)).0, 422);
        }
        for path in ["/api/posts/-1", "/api/posts/zero"] {
            assert_eq!(
                request(port, "PUT", path, Some(json!({"title":"題","body":"文"}))).0,
                422
            );
        }
        assert_eq!(
            request(
                port,
                "PUT",
                "/api/posts/999",
                Some(json!({"title":"題","body":"文"}))
            )
            .0,
            404
        );
        assert_eq!(exchange(port, b"POST /api/posts HTTP/1.1\r\nContent-Type: application/json\r\nX-Hanabi-Request: 1\r\nContent-Length: 1\r\n\r\n\xff").0, 422);
        assert_eq!(
            exchange(
                port,
                b"POST /api/posts HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}"
            )
            .0,
            422
        );
        assert_eq!(
            exchange(
                port,
                b"POST /api/posts HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n"
            )
            .0,
            400
        );
        assert_eq!(
            exchange(
                port,
                b"POST /api/posts HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"
            )
            .0,
            400
        );
        assert_eq!(request(port, "GET", "/unknown.css", None).0, 404);
        std::fs::remove_file(dir.0.join("client.wasm")).unwrap();
        assert_eq!(request(port, "GET", "/client.wasm", None).0, 404);
        std::fs::write(dir.0.join("client.wasm"), []).unwrap();
        assert_eq!(request(port, "GET", "/client.wasm", None), (200, vec![]));
        std::fs::write(dir.0.join("client.wasm"), [0, 97, 115, 109, 255]).unwrap();
        assert_eq!(
            request(port, "GET", "/client.wasm", None).1,
            [0, 97, 115, 109, 255]
        );
        drop(TcpStream::connect(("127.0.0.1", port)).unwrap());
        assert_eq!(request(port, "GET", "/api/posts", None).0, 200);
        drop(server);
        let server = start(&dir.0, port);
        let (status, body) = request(port, "GET", "/api/posts", None);
        assert_eq!(status, 200);
        let posts: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            posts,
            json!([{"id":1,"title":"編集後","body":"更新した本文"}])
        );
        let persisted: Value =
            serde_json::from_slice(&std::fs::read(dir.0.join("data/posts.json")).unwrap()).unwrap();
        assert_eq!(persisted, posts);
        std::fs::write(dir.0.join("data/posts.json"), [255]).unwrap();
        assert_eq!(request(port, "GET", "/api/posts", None).0, 500);
        drop(server);
    }
}
