import { spawnSync } from 'node:child_process';
import { makeServer } from './server.mjs';

const server = await makeServer();
const gates = [
  ['check', 'integrity'],
  ['check', 'copy', '--manifest', 'demo/copy.txt'],
  ['scan', 'scroll'],
  ['scan', 'handlers'],
  ['check', 'interactions'],
  ['check', 'breakpoints', '--sweep'],
];
try {
  for (const [verb, gate, ...options] of gates) {
    const result = spawnSync('node_modules/.bin/vlmkit', [verb, gate, server.url, ...options, '--json'], { encoding: 'utf8' });
    if (result.error) throw result.error;
    let report;
    try { report = JSON.parse(result.stdout); }
    catch { throw new Error(result.stdout + result.stderr); }
    console.log(JSON.stringify({ gate: report.gate, verdict: report.verdict, counts: report.counts, findings: report.findings }));
    if (result.status !== 0 || report.verdict !== 'pass') throw new Error(result.stdout + result.stderr);
  }
} finally { await server.close(); }
