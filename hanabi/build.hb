fn main() {
    write_bytes('target/hanabi/hanabi', compile_file('hanabi/demo/server.hb', 'native', []))
    let exports = ['start', 'submit', 'edit', 'cancel', 'receive', 'network_error']
    write_bytes('target/hanabi/client.wasm', compile_file('hanabi/demo/client.hb', 'wasm', exports))
    write_text('target/hanabi/client.mjs', js_glue(inspect_file_exports('hanabi/demo/client.hb', exports)))
    write_bytes('target/hanabi/index.html', read_bytes('hanabi/demo/index.html'))
    write_bytes('target/hanabi/style.css', read_bytes('hanabi/demo/style.css'))
    write_bytes('target/hanabi/browser.mjs', read_bytes('hanabi/browser.mjs'))
}
