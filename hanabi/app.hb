use std -> *
use './types.hb' -> *
use './storage.hb' -> { Storage }
use './server.hb'

struct Asset { url: String, path: FilePath, content_type: String }
struct Config { port: Port, directory: FilePath, assets: Vec<Asset> }
fn dispatch(request: server.Request, assets: Vec<Asset>, handler: (server.Request) -> server.Response) -> server.Response
with Storage(storage) {
    if request.method == Get {
        for file in assets { if file.url == request.path { return server.asset(file.path, file.content_type) } }
    }
    handler(request)
}
fn run(config: Config, handler: (server.Request) -> server.Response)
with Storage(storage) {
    match storage.directory(config.directory) { Ok(_) -> {} Err(e) -> panic(e) }
    server.serve(config.port, fn(request) { dispatch(request, config.assets, handler) })
}
