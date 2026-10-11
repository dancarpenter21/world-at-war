import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { options, runSoak } from './performance-soak.mjs';

test('soak options default to two one-hour modes and reject invalid settings', () => {
  const o = options([]); assert.equal(o.storage, 'both'); assert.equal(o.duration, '3600');
  for (const args of [['--storage', 'other'], ['--duration', '0'], ['--clients', '201'], ['--duration', 'NaN']]) assert.throws(() => options(args));
});

test('unattended runner preserves failed mode, continues, and refuses overwrite', async () => {
  const tmp = await mkdtemp(path.join(tmpdir(), 'waw-soak-test-'));
  try {
    const harness = path.join(tmp, 'fixture.mjs');
    await writeFile(harness, `import {mkdir,writeFile} from 'node:fs/promises';
const arg=k=>process.argv[process.argv.indexOf('--'+k)+1];const out=arg('output');await mkdir(out);
const valid=arg('storage')==='postgres';
await writeFile(out+'/metadata.json',JSON.stringify({measurement_completed:true}));
await writeFile(out+'/summary.json',JSON.stringify({valid,errors:valid?[]:['fixture failure'],budgets:{reconnect:[]},metrics:{server:{cycle_duration_ms:{p95:12,max:20}}}}));
console.log('fixture progress');process.exitCode=valid?0:1;`);
    const output = path.join(tmp, 'result');
    const opts = options(['--output', output, '--duration', '1', '--warmup', '0']);
    assert.equal(await runSoak(opts, harness), 1);
    const report = JSON.parse(await readFile(path.join(output, 'report.json')));
    assert.deepEqual(report.runs.map(r => r.state), ['failed', 'passed']);
    assert.match(await readFile(path.join(output, 'volatile.log'), 'utf8'), /fixture progress/);
    assert.match(await readFile(path.join(output, 'report.md'), 'utf8'), /Soak test: failed/);
    await assert.rejects(runSoak(opts, harness), /EEXIST/);
  } finally { await rm(tmp, { recursive: true, force: true }); }
});

test('watchdog requests cleanup and records timeout with missing artifacts', async () => {
  const tmp = await mkdtemp(path.join(tmpdir(), 'waw-soak-timeout-'));
  try {
    const harness = path.join(tmp, 'fixture.mjs');
    await writeFile(harness, `setInterval(()=>{},1000);process.on('SIGTERM',()=>{console.log('cleanup requested');process.exit(0)});`);
    const output = path.join(tmp, 'result');
    assert.equal(await runSoak(options(['--output', output, '--storage', 'volatile', '--duration', '1', '--warmup', '0', '--timeout-grace', '1']), harness), 1);
    const report = JSON.parse(await readFile(path.join(output, 'report.json')));
    assert.equal(report.runs[0].timed_out, true); assert.equal(report.state, 'failed');
    assert.match(await readFile(path.join(output, 'volatile.log'), 'utf8'), /cleanup requested/);
  } finally { await rm(tmp, { recursive: true, force: true }); }
});

test('termination forwards cleanup, records interruption and skips remaining mode', async () => {
  const tmp = await mkdtemp(path.join(tmpdir(), 'waw-soak-signal-'));
  try {
    const harness = path.join(tmp, 'fixture.mjs');
    const ready = path.join(tmp, 'ready');
    await writeFile(harness, `import {writeFile} from 'node:fs/promises';
process.on('SIGTERM',()=>{console.log('cleanup requested');process.exit(0)});
setInterval(()=>{},1000);await writeFile(${JSON.stringify(ready)},'ready');`);
    const output = path.join(tmp, 'result');
    const running = runSoak(options(['--output', output, '--duration', '1']), harness);
    for (let i = 0; i < 200; i++) {
      try { await readFile(ready); break; } catch { await new Promise(resolve => setTimeout(resolve, 10)); }
    }
    process.emit('SIGTERM');
    assert.equal(await running, 130);
    const report = JSON.parse(await readFile(path.join(output, 'report.json')));
    assert.equal(report.state, 'interrupted'); assert.equal(report.runs.length, 1);
    assert.match(await readFile(path.join(output, 'volatile.log'), 'utf8'), /cleanup requested/);
  } finally { await rm(tmp, { recursive: true, force: true }); }
});
