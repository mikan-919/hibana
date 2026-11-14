import * as fs from 'node:fs';
import path from 'node:path';
import { randomBytes, scryptSync, timingSafeEqual } from 'node:crypto';

const MAX_BYTES = 16 * 1024 * 1024;
export function validatePosts(value) {
  if (!Array.isArray(value) || value.length > 10000) throw new Error('Invalid CMS posts array');
  const ids = new Set();
  for (const post of value) {
    if (!post || typeof post !== 'object' || Array.isArray(post)
      || Object.keys(post).sort().join(',') !== 'body,id,published,title'
      || typeof post.id !== 'string' || !/^[a-zA-Z0-9_-]{1,100}$/.test(post.id)
      || ids.has(post.id) || typeof post.title !== 'string' || [...post.title].length < 1 || [...post.title].length > 200
      || typeof post.body !== 'string' || [...post.body].length > 100000
      || typeof post.published !== 'boolean') throw new Error('Invalid CMS post');
    ids.add(post.id);
  }
  return value;
}

// A single synchronous writer, protected by an exclusive directory lock.
// Never replace a corrupt database with an empty one. Stale locks require manual recovery.
export function createRuntime({ env = process.env, now = Date.now } = {}) {
  const password = env.CMS_PASSWORD;
  if (typeof password !== 'string' || password.length < 12 || password.length > 1024)
    throw new Error('CMS_PASSWORD must contain 12 to 1024 characters');
  const portText = env.PORT ?? '3000';
  if (!/^\d+$/.test(portText) || Number(portText) > 65535) throw new Error('PORT must be 0..65535');
  const dir = path.resolve(env.CMS_DATA_DIR || 'apps/cms/data');
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const lock = path.join(dir, '.cms.lock');
  try { fs.mkdirSync(lock, { mode: 0o700 }); }
  catch (error) {
    if (error.code === 'EEXIST') throw new Error('CMS data directory is locked; stop other instances before manually removing .cms.lock');
    throw error;
  }
  let closed = false;
  let poisoned = false;
  const file = path.join(dir, 'posts.json');
  const sessions = new Map();
  const salt = randomBytes(32);
  const hash = scryptSync(password, salt, 64);
  let failures = 0;
  let retryAt = 0;
  const ensureOpen = () => { if (closed || poisoned) throw new Error('CMS storage is closed or unavailable'); };
  function read() {
    let text;
    try {
      if (fs.statSync(file).size > MAX_BYTES) throw new Error('CMS database too large');
      text = fs.readFileSync(file, 'utf8');
    } catch (error) {
      if (error.code === 'ENOENT') return '[]';
      throw error;
    }
    validatePosts(JSON.parse(text));
    return text;
  }
  function close() {
    if (closed) return;
    closed = true;
    sessions.clear();
    fs.rmSync(lock, { recursive: true });
  }
  try { read(); } catch (error) { close(); throw new Error('Refusing corrupt CMS database', { cause: error }); }
  function tokenFromCookie(cookie) {
    if (typeof cookie !== 'string') return '';
    const matches = cookie.split(';').map(v => v.trim()).filter(v => v.startsWith('cms_session='));
    if (matches.length !== 1) return '';
    const token = matches[0].slice('cms_session='.length);
    return /^[a-f0-9]{64}$/.test(token) ? token : '';
  }
  function session(cookie) {
    ensureOpen();
    const token = tokenFromCookie(cookie);
    const entry = sessions.get(token);
    if (!entry) return undefined;
    if (now() >= entry.expires) { sessions.delete(token); return undefined; }
    return entry;
  }
  const globals = {
    cms_load() {
      ensureOpen();
      try { return read(); } catch (error) { poisoned = true; throw error; }
    },
    cms_save(text) {
      ensureOpen();
      if (typeof text !== 'string' || Buffer.byteLength(text) > MAX_BYTES) throw new Error('CMS database too large');
      const canonical = JSON.stringify(validatePosts(JSON.parse(text))) + '\n';
      if (Buffer.byteLength(canonical) > MAX_BYTES) throw new Error('CMS database too large');
      // Recheck disk so external corruption is not silently overwritten.
      try { read(); } catch (error) { poisoned = true; throw error; }
      const tmp = path.join(dir, `.posts-${randomBytes(16).toString('hex')}.tmp`);
      let fd;
      let renamed = false;
      try {
        fd = fs.openSync(tmp, 'wx', 0o600);
        fs.writeFileSync(fd, canonical, 'utf8');
        fs.fsyncSync(fd);
        fs.closeSync(fd); fd = undefined;
        fs.renameSync(tmp, file); renamed = true;
        const dirFd = fs.openSync(dir, 'r');
        try { fs.fsyncSync(dirFd); } finally { fs.closeSync(dirFd); }
      } catch (error) {
        // After rename a failed directory fsync has uncertain durability: fail closed.
        if (renamed) poisoned = true;
        throw error;
      } finally {
        if (fd !== undefined) fs.closeSync(fd);
        if (!renamed) fs.rmSync(tmp, { force: true });
      }
    },
    cms_login(candidate) {
      ensureOpen();
      const time = now();
      if (time < retryAt) return '';
      if (typeof candidate !== 'string' || candidate.length > 1024
        || !timingSafeEqual(hash, scryptSync(candidate, salt, 64))) {
        failures += 1;
        if (failures >= 5) { retryAt = time + 60000; failures = 0; }
        return '';
      }
      failures = 0; retryAt = 0;
      for (const [token, entry] of sessions) if (time >= entry.expires) sessions.delete(token);
      if (sessions.size >= 100) sessions.delete(sessions.keys().next().value);
      const token = randomBytes(32).toString('hex');
      sessions.set(token, { csrf: randomBytes(32).toString('hex'), expires: time + 8 * 60 * 60 * 1000 });
      return token;
    },
    cms_authorized(cookie) { return !!session(cookie); },
    cms_csrf(cookie) { return session(cookie)?.csrf ?? ''; },
    cms_logout(cookie) { ensureOpen(); sessions.delete(tokenFromCookie(cookie)); },
    cms_cookie_secure() { return env.CMS_SECURE_COOKIE === '1'; },
    cms_port() { return BigInt(portText); },
  };
  return { globals, close, dataDir: dir };
}
