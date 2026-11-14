import { createRuntime } from './runtime.mjs';

let runtime;
try {
  runtime = createRuntime();
  Object.assign(globalThis, runtime.globals);
  process.once('exit', () => runtime.close());
  // Node's HTTP server is not exported by the generated app. SIGTERM/SIGINT
  // run synchronous lock cleanup through the exit handler.
  process.once('SIGTERM', () => process.exit(0));
  process.once('SIGINT', () => process.exit(0));
  await import('./dist/app.mjs');
} catch (error) {
  runtime?.close();
  console.error(`CMS startup failed: ${error.message}`);
  process.exitCode = 1;
}
