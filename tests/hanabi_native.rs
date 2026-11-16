#![cfg(all(target_arch = "x86_64", target_os = "linux"))]
use hibana::pipeline::{self, Kind};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(dir: &Path, port: u16) -> Server {
    let mut server = Server(
        Command::new(dir.join("server"))
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    for _ in 0..300 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return server;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "サーバーが起動前に終了した"
        );
        std::thread::sleep(Duration::from_millis(10));
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
    let head = std::str::from_utf8(&output[..end]).unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
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
fn native_posts_are_validated_saved_edited_and_reloaded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let source = std::fs::read_to_string(root.join("hanabi/demo/server.hb"))
        .unwrap()
        .replace("run(8787, app)", &format!("run({port}, app)"));
    let expanded = pipeline::load(&source, &root.join("hanabi/demo")).unwrap();
    let checked = pipeline::check(pipeline::parse(&expanded).unwrap(), Kind::Executable).unwrap();
    let ir = pipeline::lower(&checked, Some(&["main".into()])).unwrap();
    let bytes = hibana::native::emit(&ir).unwrap();
    let dir = std::env::temp_dir().join(format!("hanabi-native-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("server"), bytes).unwrap();
    std::fs::set_permissions(dir.join("server"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("index.html"), "hanabi").unwrap();
    std::fs::write(dir.join("client.wasm"), [0, 97, 115, 109, 255]).unwrap();
    drop(listener);
    let server = start(&dir, port);
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
    assert_eq!(post["title"], "<script>題</script>");
    let (status, body) = request(
        port,
        "PUT",
        "/api/posts",
        Some(json!({"id":1,"title":"編集後","body":"更新した本文"})),
    );
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        request(
            port,
            "POST",
            "/api/posts",
            Some(json!({"title":"","body":"本文"}))
        )
        .0,
        422
    );
    assert_eq!(
        request(
            port,
            "POST",
            "/api/posts",
            Some(json!({"title":"x".repeat(121),"body":"本文"}))
        )
        .0,
        422
    );
    assert_eq!(
        request(
            port,
            "PUT",
            "/api/posts",
            Some(json!({"id":-1,"title":"題","body":"文"}))
        )
        .0,
        422
    );
    assert_eq!(
        request(
            port,
            "PUT",
            "/api/posts",
            Some(json!({"id":999,"title":"題","body":"文"}))
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
    assert_eq!(request(port, "GET", "/unknown", None).0, 404);
    assert_eq!(
        request(port, "GET", "/client.wasm", None).1,
        [0, 97, 115, 109, 255]
    );
    drop(server);
    let server = start(&dir, port);
    let (status, body) = request(port, "GET", "/api/posts", None);
    assert_eq!(status, 200);
    let posts: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(posts.as_array().unwrap().len(), 1);
    assert_eq!(posts[0]["title"], "編集後");
    let persisted: Value =
        serde_json::from_slice(&std::fs::read(dir.join("data/posts.json")).unwrap()).unwrap();
    assert_eq!(persisted, posts);
    std::fs::write(dir.join("data/posts.json"), [255]).unwrap();
    assert_eq!(request(port, "GET", "/api/posts", None).0, 500);
    drop(server);
    std::fs::remove_dir_all(dir).unwrap();
}
