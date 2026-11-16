// DOM と非同期通信の接続。入力検証と表示の生成は WASM に委ねる
import { instantiate } from './client.mjs';
const state = new Map();
let api;
let pending = false;
const element = selector => {
  const node = document.querySelector(selector);
  if (!node) throw new Error(`要素が見つかりません: ${selector}`);
  return node;
};
const host = {
  render: (selector, html) => { element(selector).innerHTML = html; },
  text: (selector, text) => { element(selector).textContent = text; },
  value: selector => element(selector).value,
  set_value: (selector, value) => { element(selector).value = value; },
  focus: selector => element(selector).focus(),
  state: key => state.get(key) ?? '',
  set_state: (key, value) => { state.set(key, value); },
  busy: (selector, value) => {
    const root = element(selector);
    root.querySelectorAll('button').forEach(node => { node.disabled = Number(value) !== 0; });
    root.setAttribute('aria-busy', String(Number(value) !== 0));
  },
  notice: (selector, message, error) => {
    const notice = element(selector);
    notice.textContent = message;
    notice.dataset.error = String(Number(error) !== 0);
  },
  request: (method, path, body) => {
    if (pending) return;
    pending = true;
    fetch(path, { method, headers: { 'Content-Type': 'application/json', 'X-Hanabi-Request': '1' }, body: method === 'GET' ? undefined : body })
      .then(async response => { const text = await response.text(); pending = false; api.receive(response.status, text); })
      .catch(() => { pending = false; api.network_error('通信に失敗しました。もう一度操作してください。'); });
  },
};
document.addEventListener('submit', event => {
  const form = event.target.closest('[data-hanabi-submit]');
  if (!form) return;
  event.preventDefault();
  if (!pending) api?.[form.dataset.hanabiSubmit]?.();
});
document.addEventListener('click', event => {
  const button = event.target.closest('button[data-hanabi-call]');
  if (!button || pending) return;
  const handler = api?.[button.dataset.hanabiCall];
  if (typeof handler !== 'function') return;
  if (button.dataset.id === undefined) handler();
  else if (/^[1-9][0-9]*$/.test(button.dataset.id)) handler(BigInt(button.dataset.id));
});
try {
  const response = await fetch('./client.wasm');
  if (!response.ok) throw new Error('画面を読み込めませんでした');
  const functions = Object.fromEntries(Object.entries(host).map(([name, callback]) => [`dom.${name}`, callback]));
  api = await instantiate(await response.arrayBuffer(), { hibana_host: functions });
  api.start();
} catch (error) {
  element('#app').textContent = '画面を読み込めませんでした。ページを再読み込みしてください。';
  console.error(error);
}
