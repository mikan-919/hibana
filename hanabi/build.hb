use std -> *
use './types.hb' -> { Port }

struct File { source: String, destination: String }
struct Build {
    output: String, service: String, module_name: String, entry: String, port: Port,
    client: String, exports: Vec<String>, files: Vec<File>
}
fn build(config: Build) {
    if includes(config.service, "'") || includes(config.service, '\n') { panic('サービスの経路に引用符や改行は使えない') }
    let entry = "use '" + config.service + "'\nfn main() { " + config.module_name + '.' + config.entry + '(' + config.port.to_string() + ') }\n'
    write_text(config.output + '/entry.hb', entry)
    write_bytes(config.output + '/application', compile_file(config.output + '/entry.hb', 'native', []))
    write_bytes(config.output + '/client.wasm', compile_file(config.client, 'wasm', config.exports))
    write_text(config.output + '/client.mjs', js_glue(inspect_file_exports(config.client, config.exports)))
    for file in config.files { write_bytes(config.output + '/' + file.destination, read_bytes(file.source)) }
}
