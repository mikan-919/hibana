// WASM の型付きセッションへ DOM イベントと通信完了を配送する。
export async function mount(config) {
  const { instantiate } = await import(config.glue);
  const response = await fetch(config.wasm);
  if (!response.ok) throw new Error('WASM を読み込めない');
  const element = selector => {
    const node = document.querySelector(selector);
    if (!node) throw new Error(`要素が見つからない: ${selector}`);
    return node;
  };
  let api;
  let session;
  let disposed = false;
  const controller = new AbortController();
  const host = {
    'dom.read_value': selector => element(selector).value,
    'dom.render': (selector, html) => { element(selector).innerHTML = html; },
    'dom.focus_element': selector => element(selector).focus(),
    'dom.schedule': (id, method, url, body) => {
      queueMicrotask(async () => {
        let status;
        let text;
        try {
          const reply = await fetch(url, {
            method, headers: { 'Content-Type': 'application/json', 'X-Hanabi-Request': '1' },
            body: method === 'GET' ? undefined : body, signal: controller.signal,
          });
          status = reply.status;
          text = await reply.text();
        } catch (error) {
          if (disposed) return;
          // ネットワーク障害も型付きのエラーへ変換する。プロトコル外の応答本文は渡さない。
          status = 503;
          text = '';
        }
        if (!disposed) api[config.resolve](session, id, status, text);
      });
    },
  };
  api = await instantiate(await response.arrayBuffer(), { hibana_host: host });
  session = api[config.entry](config.root);
  const root = element(config.root);
  const handlers = ['click', 'submit', 'change'].map(kind => {
    const listener = event => {
      if (!(event.target instanceof Element)) return;
      const node = event.target.closest(`[data-hanabi-${kind}]`);
      if (!node || !root.contains(node) || node.disabled) return;
      event.preventDefault();
      api[config.dispatch](session, node.getAttribute(`data-hanabi-${kind}`));
    };
    root.addEventListener(kind, listener);
    return [kind, listener];
  });
  return { dispose() {
    disposed = true;
    controller.abort();
    for (const [kind, listener] of handlers) root.removeEventListener(kind, listener);
    api.dispose();
  } };
}
