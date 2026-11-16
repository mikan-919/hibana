use std -> *
use '../build.hb' -> { Build, File, build }
use '../types.hb' -> { Port }

fn main() {
    let options = args()
    if len(options) < 1 { panic('リポジトリの絶対経路を第1引数に指定する') }
    let root = options[0]
    let output = if len(options) > 1 { options[1] } else { root + '/target/hanabi' }
    let port: Port = if len(options) > 2 { match refine<Port>(match parse_int(options[2]) { Ok(n) -> n Err(e) -> panic(e) }) { Ok(port) -> port Err(e) -> panic(e) } } else { 8787 }
    build(Build {
        output: output, service: root + '/hanabi/demo/service.hb', module_name: 'service', entry: 'serve', port: port,
        client: root + '/hanabi/demo/client.hb', exports: ['bootstrap', 'dispatch', 'resolve'],
        files: [
            File { source: root + '/hanabi/demo/index.html', destination: 'index.html' },
            File { source: root + '/hanabi/demo/style.css', destination: 'style.css' },
            File { source: root + '/hanabi/browser.mjs', destination: 'browser.mjs' },
            File { source: root + '/hanabi/demo/main.mjs', destination: 'main.mjs' }
        ]
    })
}
