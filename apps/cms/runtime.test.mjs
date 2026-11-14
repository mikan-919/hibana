import test from 'node:test';
import assert from 'node:assert/strict';
import * as fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRuntime } from './runtime.mjs';

const password = 'local-testing-password';
function setup(t, extra = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'hibana-cms-runtime-'));
  const env = { CMS_PASSWORD: password, CMS_DATA_DIR: dir, PORT: '0', ...extra };
  const runtimes = [];
  t.after(() => {
    try { for (const runtime of runtimes) runtime.close(); }
    finally { fs.rmSync(dir, { recursive: true, force: true }); }
  });
  return { dir, env, open(options = {}) { const r = createRuntime({ env, ...options }); runtimes.push(r); return r; } };
}
const post = { id: '1', title: '日本語 & <title>', body: 'line 1\nline 2\t\r\u0000', published: false };

test('atomic durable JSON, restart persistence and lock exclusion', t => {
  const ctx = setup(t);
  const first = ctx.open();
  assert.equal(first.globals.cms_load(), '[]');
  assert.throws(() => ctx.open(), /locked/);
  first.globals.cms_save(JSON.stringify([post]));
  assert.deepEqual(JSON.parse(first.globals.cms_load()), [post]);
  assert.equal(fs.statSync(path.join(ctx.dir, 'posts.json')).mode & 0o777, 0o600);
  assert.equal(fs.readdirSync(ctx.dir).filter(n => n.endsWith('.tmp')).length, 0);
  first.close();
  const second = ctx.open();
  assert.deepEqual(JSON.parse(second.globals.cms_load()), [post]);
  assert.equal(second.globals.cms_port(), 0n);
});

test('invalid data never overwrites the existing database', t => {
  const ctx = setup(t); const r = ctx.open();
  r.globals.cms_save(JSON.stringify([post]));
  for (const value of ['{', '{}', '[null]', JSON.stringify([post, post]), JSON.stringify([{ ...post, published: 'yes' }])])
    assert.throws(() => r.globals.cms_save(value));
  assert.deepEqual(JSON.parse(r.globals.cms_load()), [post]);
});

test('corrupt storage fails startup, preserving evidence and releasing its lock', t => {
  const ctx = setup(t);
  const file = path.join(ctx.dir, 'posts.json');
  fs.writeFileSync(file, 'broken json');
  assert.throws(() => ctx.open(), /corrupt/);
  assert.equal(fs.readFileSync(file, 'utf8'), 'broken json');
  assert.equal(fs.existsSync(path.join(ctx.dir, '.cms.lock')), false);
});

test('runtime refuses external corruption instead of overwriting it', t => {
  const ctx = setup(t); const r = ctx.open();
  fs.writeFileSync(path.join(ctx.dir, 'posts.json'), 'broken');
  assert.throws(() => r.globals.cms_save('[]'));
  assert.throws(() => r.globals.cms_load(), /unavailable/);
  assert.equal(fs.readFileSync(path.join(ctx.dir, 'posts.json'), 'utf8'), 'broken');
});

test('password authentication, cookie parsing, logout, CSRF and fixed expiry', t => {
  const ctx = setup(t, { CMS_SECURE_COOKIE: '1' });
  let time = 1000;
  const r = ctx.open({ now: () => time }); const g = r.globals;
  assert.equal(g.cms_login('incorrect'), '');
  const token = g.cms_login(password); const cookie = `other=1; cms_session=${token}`;
  assert.match(token, /^[a-f0-9]{64}$/);
  assert.equal(g.cms_authorized(cookie), true);
  assert.match(g.cms_csrf(cookie), /^[a-f0-9]{64}$/);
  assert.notEqual(g.cms_csrf(cookie), token);
  assert.equal(g.cms_authorized(`${cookie}; cms_session=${token}`), false);
  assert.equal(g.cms_authorized(`cms_session=${token}x`), false);
  const second = g.cms_login(password);
  assert.notEqual(g.cms_csrf(`cms_session=${second}`), g.cms_csrf(cookie));
  g.cms_logout(cookie);
  assert.equal(g.cms_authorized(cookie), false);
  time += 8 * 60 * 60 * 1000;
  assert.equal(g.cms_csrf(`cms_session=${second}`), '');
  assert.equal(g.cms_cookie_secure(), true);
  r.close(); assert.throws(() => g.cms_login(password), /closed/);
});

test('login failures impose a global cooldown without blocking forever', t => {
  const ctx = setup(t); let time = 0;
  const g = ctx.open({ now: () => time }).globals;
  for (let i = 0; i < 5; i++) assert.equal(g.cms_login('wrong'), '');
  assert.equal(g.cms_login(password), '');
  time = 60000;
  assert.notEqual(g.cms_login(password), '');
});

test('configuration rejects weak passwords and invalid ports', t => {
  const ctx = setup(t);
  for (const CMS_PASSWORD of [undefined, '', 'short'])
    assert.throws(() => createRuntime({ env: { ...ctx.env, CMS_PASSWORD } }), /CMS_PASSWORD/);
  for (const PORT of ['-1', '65536', 'abc', '1.5'])
    assert.throws(() => createRuntime({ env: { ...ctx.env, PORT } }), /PORT/);
});
