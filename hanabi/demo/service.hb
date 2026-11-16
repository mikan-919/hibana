use '../app.hb' -> { Config, Asset, run }
use '../server.hb' -> { Request, Response }
use '../routes.hb'
use '../storage.hb' -> { Storage, NativeStorage }
use '../types.hb' -> { Port }
use './api.hb' -> *
use './stores.hb' -> { Repository }

fn endpoints(request: Request) -> Response
with Storage(storage) {
    let repo = Repository { path: 'data/posts.json' }
    routes request {
        list_api -> fn(_, _) { stores.list(repo) };
        create_api -> fn(_, form) { stores.create(repo, form) };
        update_api -> fn(id, form) { stores.update(repo, id, form) }
    }
}
fn serve(port: Port) {
    with Storage(NativeStorage {}) {
        run(Config {
            port: port, directory: 'data',
            assets: [
                Asset { url: '/', path: 'index.html', content_type: 'text/html; charset=utf-8' },
                Asset { url: '/style.css', path: 'style.css', content_type: 'text/css; charset=utf-8' },
                Asset { url: '/browser.mjs', path: 'browser.mjs', content_type: 'text/javascript; charset=utf-8' },
                Asset { url: '/client.mjs', path: 'client.mjs', content_type: 'text/javascript; charset=utf-8' },
                Asset { url: '/main.mjs', path: 'main.mjs', content_type: 'text/javascript; charset=utf-8' },
                Asset { url: '/client.wasm', path: 'client.wasm', content_type: 'application/wasm' }
            ]
            }, fn(request) { endpoints(request) })
    }
}
