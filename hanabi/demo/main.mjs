import { mount } from './browser.mjs';
await mount({ wasm: './client.wasm', glue: './client.mjs', root: '#app', entry: 'bootstrap', dispatch: 'dispatch', resolve: 'resolve' });
