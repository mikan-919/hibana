import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, chmodSync, existsSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'node:net';
import { once } from 'node:events';
const root = fileURLToPath(new URL('../../', import.meta.url));
export async function makeServer() {
  const socket = createServer().listen(0, '127.0.0.1');
  await once(socket, 'listening');
  const port = socket.address().port;
  await new Promise(resolve => socket.close(resolve));
  const dir = mkdtempSync(join(tmpdir(), 'hanabi-browser-'));
  const output = dir;
  const build = spawnSync(join(root, 'target/debug/hibana'), [join(root, 'hanabi/demo/build.hb'), '--', root, output, String(port)], { cwd: root, encoding: 'utf8' });
  if (build.status !== 0) {
    rmSync(dir, { recursive: true, force: true });
    throw new Error(build.stdout + build.stderr);
  }
  for (const name of ['application', 'client.wasm', 'client.mjs', 'browser.mjs', 'main.mjs', 'index.html', 'style.css']) {
    if (!existsSync(join(output, name))) throw new Error(`生成物が無い: ${name}`);
  }
  chmodSync(join(dir, 'application'), 0o755);
  let child;
  let logs = '';
  const url = `http://127.0.0.1:${port}`;
  async function start() {
    child = spawn(join(dir, 'application'), [], { cwd: dir, stdio: ['ignore', 'pipe', 'pipe'] });
    child.stdout.on('data', chunk => { logs += chunk; });
    child.stderr.on('data', chunk => { logs += chunk; });
    for (let i = 0; i < 100; i++) {
      if (child.exitCode !== null) throw new Error(`サーバーが終了した: ${logs}`);
      try { if ((await fetch(url)).ok) return; } catch {}
      await new Promise(resolve => setTimeout(resolve, 30));
    }
    throw new Error(`サーバーが起動しない: ${logs}`);
  }
  async function stop() {
    if (child && child.exitCode === null && child.signalCode === null) {
      const exited = once(child, 'exit');
      child.kill('SIGTERM');
      await exited;
    }
  }
  try { await start(); }
  catch (error) { await stop(); rmSync(dir, { recursive: true, force: true }); throw error; }
  return {
    url, dir, output,
    persisted: () => JSON.parse(readFileSync(join(dir, 'data/posts.json'), 'utf8')),
    restart: async () => { await stop(); await start(); },
    close: async () => { await stop(); rmSync(dir, { recursive: true, force: true }); },
  };
}
