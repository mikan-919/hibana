use build_web

// 出力名と公開する関数は、ライブラリではなく利用側が選ぶ。
web_build "app.wasm", "app.mjs" exports (greet, add)
build_asset "index.html", "<!doctype html><meta charset='utf-8'><script type='module'>import { instantiate } from './app.mjs'; const app = await instantiate(await (await fetch('./app.wasm')).arrayBuffer()); document.body.textContent = app.greet('hibana');</script>"

fn greet(name: str) -> str { name }
fn add(a: int, b: int) -> int { a + b }
