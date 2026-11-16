use hibana::pipeline::{self, Kind};
use std::path::Path;

fn check(source: &str) -> hibana::error::R<pipeline::Checked> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("hanabi");
    let expanded = pipeline::load(source, &root)?;
    pipeline::check(pipeline::parse(&expanded)?, Kind::Executable)
}

#[test]
fn hanabi_contract_uses_refinements_capabilities_effects_and_views() {
    let source = "use './demo/domain.hb' -> *\nuse './demo/stores.hb' -> { MemoryPosts }\nuse './view.hb' -> { text, render }\nfn main() { handle { with Posts(MemoryPosts { posts: new_ref([]) }) { let post = create_post(draft('タイトル', '本文'))\n put(post.title)\n put(len(list_posts()))\n update_post(post.id, draft('編集', '更新した本文'))\n for p in list_posts() { put(p.title) } } } { fail(error) -> { put(error) } }\n put(render(view h3 { text('<script>') })) }";
    let checked = check(source).unwrap();
    let mut output = Vec::new();
    hibana::eval::run(checked.program(), &mut output).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "タイトル\n1\n編集\n<h3>&lt;script&gt;</h3>\n"
    );
    let ir = pipeline::lower(&checked, Some(&["main".into()])).unwrap();
    assert!(pipeline::emit_wasm(&ir, Some(&["main".into()])).is_ok());
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    assert!(hibana::native::emit(&ir).is_ok());
}

#[test]
fn hanabi_rejects_missing_context_effects_and_invalid_contracts() {
    for (source, message) in [
        ("use './demo/domain.hb' -> *\nfn handler() ! Failure { list_posts() }\nfn main() {}", "with domain.Posts が必要"),
        ("use './demo/domain.hb' -> *\nuse './demo/stores.hb' -> { MemoryPosts }\nfn main() { with Posts(MemoryPosts { posts: new_ref([]) }) { create_post(draft('題', '文')) } }", "効果"),
        ("use './demo/domain.hb' -> *\nfn main() { let title: Title = '' }", "証明できない"),
        ("use './view.hb' -> { render }\nfn main() { put(render(view div { '<script>' })) }", "view.Html"),
        ("use './routes.hb'\nuse './server.hb' -> *\nfn app(req: Request) -> Response { routes req { GET '/x' -> { text(200, 'a') } GET '/x' -> { text(200, 'b') } } }\nfn main() {}", "経路が重複"),
    ] {
        let error = match check(source) { Ok(_) => panic!("拒否しなかった: {source}"), Err(e) => e.to_string() };
        assert!(error.contains(message), "{error}");
    }
}

#[test]
fn native_host_contract_checks_signatures_and_rejects_wasm() {
    let checked = check("use './native.hb'\nfn main() { put(native.listen(8787)) }").unwrap();
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
        let error = pipeline::lower(&checked, Some(&["main".into()])).unwrap_err();
        assert!(error.to_string().contains("hibana:native"));
    }
}
