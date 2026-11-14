use std::process::Command;

/// Exercise the actual generated Hibana app over HTTP, not a replacement handler.
#[test]
fn cms_http_auth_crud_and_restart() {
    let dir = std::env::temp_dir().join(format!("hibana-cms-e2e-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("dist")).unwrap();
    let javascript = hibana::to_js(include_str!("../apps/cms/main.hb"))
        .expect("CMS must compile through the public JS API");
    std::fs::write(dir.join("dist/app.mjs"), javascript).unwrap();
    for name in ["runtime.mjs", "start.mjs"] {
        std::fs::copy(format!("apps/cms/{name}"), dir.join(name)).unwrap();
    }
    std::fs::write(dir.join("check.mjs"), HARNESS).unwrap();
    let result = Command::new("node")
        .arg(dir.join("check.mjs"))
        .current_dir(&dir)
        .output()
        .expect("Node.js is required for CMS integration tests");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.status.success(),
        "CMS HTTP integration failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

const HARNESS: &str = r#"
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { resolve } from 'node:path';
let child;
let base;
let diagnostics = '';
const deadline = setTimeout(() => { child?.kill('SIGKILL'); console.error('CMS test timed out', diagnostics); process.exit(1); }, 45000);
async function start() {
  child = spawn(process.execPath, ['start.mjs'], { env: { ...process.env, CMS_PASSWORD: 'integration-test-password', CMS_DATA_DIR: resolve('data'), PORT: '0' }, stdio: ['ignore', 'pipe', 'pipe'] });
  child.stderr.on('data', b => { diagnostics += b; });
  base = await new Promise((ok, fail) => {
    let output = '';
    child.stdout.on('data', b => { output += b; const match = output.match(/http:\/\/localhost:\d+/); if (match) ok(match[0]); });
    child.once('error', fail);
    child.once('exit', (code, signal) => fail(Error(`CMS exited before listening: ${code}/${signal}: ${diagnostics}`)));
  });
}
async function stop() {
  const exited = once(child, 'exit');
  child.kill('SIGTERM');
  await exited;
  child = undefined;
}
async function get(path, cookie = '') {
  return fetch(base + path, { headers: { cookie }, redirect: 'manual' });
}
async function post(path, fields, cookie = '') {
  return fetch(base + path, { method: 'POST', headers: { cookie, 'content-type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams(fields), redirect: 'manual' });
}
async function login() {
  const response = await post('/login', { password: 'integration-test-password' });
  assert.equal(response.status, 303);
  assert.equal(response.headers.get('location'), '/admin');
  const header = response.headers.get('set-cookie');
  assert.match(header, /HttpOnly/i);
  assert.match(header, /SameSite=Strict/i);
  return header.split(';')[0];
}
async function csrf(cookie) {
  const response = await get('/admin/new', cookie);
  assert.equal(response.status, 200);
  const html = await response.text();
  const match = html.match(/name="_csrf"\s+value="([^"]+)"/);
  assert.ok(match, 'admin form must contain CSRF token');
  return match[1];
}
try {
  await start();
  assert.equal((await get('/')).status, 200);
  assert.notEqual((await get('/admin')).status, 200);
  assert.notEqual((await post('/admin/posts', { title: 'unauthorized', body: 'no' })).status, 303);
  assert.notEqual((await post('/login', { password: 'wrong-password' })).status, 303);
  let cookie = await login();
  let token = await csrf(cookie);
  assert.equal((await post('/admin/posts', { title: 'forged', body: 'no', _csrf: 'incorrect' }, cookie)).status, 403);
  const title = '<script>alert("x")</script> 日本語 & 🌸';
  const body = '本文\n<script>danger()</script>\t行\r末';
  let response = await post('/admin/posts', { title, body, _csrf: token }, cookie);
  assert.equal(response.status, 303);
  assert.equal(response.headers.get('location'), '/admin');
  let html = await (await get('/admin', cookie)).text();
  const match = html.match(/href="\/admin\/posts\/(\d+)"/);
  assert.ok(match, 'new draft must appear in admin list');
  const id = match[1];
  assert.equal((await get('/posts/' + id)).status, 404);
  assert.ok(!(await (await get('/')).text()).includes('alert'));
  response = await post('/admin/posts/' + id, { title, body, published: 'on', _csrf: token }, cookie);
  assert.equal(response.status, 303);
  html = await (await get('/posts/' + id)).text();
  assert.ok(html.includes('&lt;script&gt;'));
  assert.ok(html.includes('日本語'));
  assert.ok(!html.includes('<script>'));
  assert.ok(!html.includes('danger()</script>'));
  assert.ok((await (await get('/')).text()).includes('日本語'));
  // Malformed URL encoding must not mutate stored content.
  response = await fetch(base + '/admin/posts/' + id, { method: 'POST', headers: { cookie, 'content-type': 'application/x-www-form-urlencoded' }, body: '_csrf=' + token + '&title=%FF&body=bad', redirect: 'manual' });
  assert.equal(response.status, 400);
  await stop();
  await start();
  html = await (await get('/posts/' + id)).text();
  assert.ok(html.includes('日本語'), 'published article survives process restart');
  assert.notEqual((await get('/admin', cookie)).status, 200, 'sessions are not persisted');
  cookie = await login();
  token = await csrf(cookie);
  response = await post('/admin/posts/' + id + '/delete', { _csrf: token }, cookie);
  assert.equal(response.status, 303);
  assert.equal((await get('/posts/' + id)).status, 404);
  response = await post('/logout', { _csrf: token }, cookie);
  assert.equal(response.status, 303);
  assert.notEqual((await get('/admin', cookie)).status, 200);
  await stop();
  await start();
  assert.equal((await get('/posts/' + id)).status, 404, 'deletion survives restart');
  await stop();
  console.log('CMS HTTP auth/CSRF/CRUD/escaping/restart passed');
} catch (error) {
  console.error(error, diagnostics);
  process.exitCode = 1;
} finally {
  clearTimeout(deadline);
  if (child) await stop();
}
"#;
