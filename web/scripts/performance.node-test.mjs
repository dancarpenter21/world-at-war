import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { Metric, profiles, analyze } from './performance-lib.mjs';

test('histograms aggregate nearest-rank percentiles, empty input, and overflow', () => {
  const metric = new Metric(); assert.equal(metric.summary().p95, null);
  for (let i = 1; i <= 100; i++) metric.add(i);
  assert.equal(metric.summary().p95, 95); assert.equal(metric.summary().mean, 50.5);
  const bounded = new Metric(1, 10); for (let i = 1; i < 1000; i++) bounded.add(i);
  assert.equal(bounded.bins.size, 11); assert.equal(bounded.summary().max, 999);
  assert.throws(() => bounded.add(NaN));
});
test('reference profiles use distinct scenario seats and configurable player counts', () => {
  assert.deepEqual(profiles('mixed').map(([, n]) => n), [11, 11, 11, 13, 4]);
  assert.equal(profiles('regional', 50).reduce((sum, [, n]) => sum + n, 0), 50);
  assert.throws(() => profiles('unknown')); assert.throws(() => profiles('mixed', 0));
});
test('analysis excludes warmup and detects incomplete or dropped traces', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'waw-perf-test-'));
  try {
    const server = [
      { kind: 'trace_start', unix_ms: 1000, elapsed_ms: 0 },
      { kind: 'tick', game_id: 'a', duration_ms: 900, elapsed_ms: 10 },
      { kind: 'tick', game_id: 'a', duration_ms: 5, elapsed_ms: 100 },
      { kind: 'trace_end', written: 3, dropped: 0 },
    ];
    const save = () => writeFile(path.join(directory, 'server.jsonl'), server.map(r => JSON.stringify(r)).join('\n'));
    await save(); await writeFile(path.join(directory, 'clients.jsonl'), '');
    let result = await analyze(directory, 1050, 1200);
    assert.equal(result.valid, true); assert.equal(result.metrics['game:a'].tick_duration_ms.max, 5);
    server.at(-1).dropped = 1; await save(); assert.equal((await analyze(directory, 1050, 1200)).valid, false);
    server.pop(); await save(); assert.equal((await analyze(directory, 1050, 1200)).valid, false);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('interrupted runs stop their isolated server and retain invalid evidence', { timeout: 30_000 }, async t => {
  const { spawn } = await import('node:child_process');
  const { once } = await import('node:events');
  const { access, readFile } = await import('node:fs/promises');
  const { fileURLToPath } = await import('node:url');
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  try { await access(path.join(root, 'target/release/world-at-war-server')); }
  catch { t.skip('Build the release server to exercise process cleanup'); return; }
  if (process.platform !== 'linux') { t.skip('Load harness process telemetry requires Linux'); return; }
  const directory = await mkdtemp(path.join(tmpdir(), 'waw-perf-cleanup-'));
  const output = path.join(directory, 'run');
  const child = spawn(process.execPath, [path.join(root, 'web/scripts/performance-server.mjs'),
    '--profile', 'regional', '--clients', '1', '--warmup', '0', '--duration', '60', '--output', output],
  { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  const exited = once(child, 'exit');
  let text = '', started = false;
  child.stdout.on('data', chunk => { text += chunk; if (!started && text.includes('Performance regional:')) { started = true; child.kill('SIGTERM'); } });
  child.stderr.on('data', chunk => { text += chunk; });
  const timeout = setTimeout(() => child.kill('SIGKILL'), 20_000);
  try {
    const [code] = await exited;
    assert.equal(started, true, text); assert.equal(code, 1, text);
    const summary = JSON.parse(await readFile(path.join(output, 'summary.json'), 'utf8'));
    assert.equal(summary.valid, false); assert.ok(summary.errors.includes('Run terminated'));
    const first = JSON.parse((await readFile(path.join(output, 'server.jsonl'), 'utf8')).split('\n')[0]);
    assert.throws(() => process.kill(first.pid, 0), { code: 'ESRCH' });
    assert.equal(summary.trace.dropped, 0);
  } finally { clearTimeout(timeout); child.kill('SIGTERM'); await rm(directory, { recursive: true, force: true }); }
});
