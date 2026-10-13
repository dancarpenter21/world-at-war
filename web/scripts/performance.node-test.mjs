import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { Metric, profiles, analyze } from './performance-lib.mjs';

async function installTerrain(directory) {
  const { createHash } = await import('node:crypto');
  const root = path.join(directory, 'terrain'); await mkdir(root);
  const data = Buffer.alloc(1201 * 1201 * 2);
  const sha256 = createHash('sha256').update(data).digest('hex');
  const tiles = [];
  for (const latitude of [37, 38]) for (const longitude of [-78, -77, -76]) {
    const file = `N${latitude}W${Math.abs(longitude).toString().padStart(3, '0')}.hgt`;
    await writeFile(path.join(root, file), data);
    tiles.push({ file, latitude, longitude, samples: 1201, sha256 });
  }
  await writeFile(path.join(root, 'manifest.json'), JSON.stringify({ version: 1,
    source: 'Synthetic flat terrain for isolated harness tests; not real SRTM acceptance evidence',
    vertical_datum: 'EGM96', tiles }));
  return root;
}

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
test('retention analysis preserves counters, peaks and sampling windows without requiring legacy traces to contain them', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'waw-retention-test-'));
  try {
    const row = (elapsed_ms, entries) => ({ kind: 'retention', game_id: 'a', elapsed_ms, tick: elapsed_ms,
      simulation: { network: { interference_entries: entries } }, server: { intent_receipts: 2 } });
    const server = [{ kind: 'trace_start', unix_ms: 1000, elapsed_ms: 0 }, row(10, 900), row(100, 20), row(150, 5),
      { kind: 'trace_end', written: 4, dropped: 0 }];
    await writeFile(path.join(directory, 'server.jsonl'), server.map(JSON.stringify).join('\n'));
    await writeFile(path.join(directory, 'clients.jsonl'), '');
    const result = await analyze(directory, 1050, 1200);
    assert.equal(result.valid, true);
    const counts = result.retention['game:a'];
    assert.equal(counts.samples, 2);
    assert.equal(counts.first.counts['simulation.network.interference_entries'], 20);
    assert.equal(counts.last.counts['simulation.network.interference_entries'], 5);
    assert.equal(counts.peak['simulation.network.interference_entries'], 20);
    assert.equal(counts.series.length, 2);
  } finally { await rm(directory, { recursive: true, force: true }); }
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
  const terrain = await installTerrain(directory);
  const child = spawn(process.execPath, [path.join(root, 'web/scripts/performance-server.mjs'),
    '--profile', 'regional', '--clients', '1', '--warmup', '0', '--duration', '60', '--output', output],
  { cwd: root, env: { ...process.env, TERRAIN_DIR: terrain }, stdio: ['ignore', 'pipe', 'pipe'] });
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

test('durable cycle analysis excludes idle cycles, retains stages, and rejects failed commits', async () => {
  const { validateRun } = await import('./performance-lib.mjs');
  const directory = await mkdtemp(path.join(tmpdir(), 'waw-durable-metrics-'));
  try {
    const server = [
      { kind: 'trace_start', unix_ms: 1000, elapsed_ms: 0 },
      { kind: 'server_cycle', success: true, running_games: [], duration_ms: 1, elapsed_ms: 100 },
      { kind: 'server_cycle', success: true, running_games: ['a'], duration_ms: 85, scheduled_to_committed_ms: 95, elapsed_ms: 200 },
      { kind: 'persistence_commit', success: true, duration_ms: 70, snapshot_submitted_bytes: 400, audit_events: 9, audit_bytes: 700, elapsed_ms: 190 },
      { kind: 'trace_end', written: 4, dropped: 0 },
    ];
    const save = () => writeFile(path.join(directory, 'server.jsonl'), server.map(JSON.stringify).join('\n'));
    await save();
    await writeFile(path.join(directory, 'clients.jsonl'), JSON.stringify({ kind: 'database_resource', time: 1150, database_bytes: 1234 }));
    const metadata = { measurement_version: 2, storage: 'postgres', games: [{ id: 'a' }], measurement_completed: true };
    let result = await analyze(directory, 1050, 1300);
    validateRun(result, metadata);
    assert.equal(result.valid, true);
    assert.equal(result.metrics.server.cycle_duration_ms.samples, 1);
    assert.equal(result.metrics.server.cycle_duration_ms.p95, 85);
    assert.equal(result.metrics.server.persistence_commit_duration_ms.max, 70);
    assert.equal(result.committed_games.a, 1);
    assert.equal(result.audit.events, 9);
    assert.equal(result.audit.bytes, 700);
    server[3].success = false; await save();
    result = await analyze(directory, 1050, 1300); validateRun(result, metadata);
    assert.equal(result.valid, false);
    assert.ok(result.errors.includes('Failed server operation: persistence_commit'));
    assert.ok(result.errors.includes('Missing PostgreSQL commit telemetry'));
    result = await analyze(directory, 1050, 1300);
    validateRun(result, { ...metadata, measurement_completed: false, games: [{ id: 'missing' }] });
    assert.ok(result.errors.includes('Measurement window did not complete'));
    assert.ok(result.errors.includes('Missing committed progress: missing'));
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('PostgreSQL startup failure cleans up only owned resources and does not expose credentials', async () => {
  const { PerformancePostgres } = await import('./performance-postgres.mjs');
  const calls = [];
  const db = new PerformancePostgres('test-image', async (args, options) => {
    calls.push({ args, options });
    if (args[0] === 'image') return JSON.stringify([{ Id: 'sha256:test', RepoDigests: ['test@sha256:test'] }]);
    if (args[0] === 'run') throw new Error('Docker run failed');
    if (args[0] === 'ps') return '';
    if (args[0] === 'volume' && args[1] === 'ls') return db.volume;
    return '';
  });
  await assert.rejects(db.start(), { message: 'Docker run failed' });
  await db.stop();
  assert.ok(calls.some(c => c.args[0] === 'volume' && c.args[1] === 'rm' && c.args[2] === db.volume));
  assert.ok(!JSON.stringify(calls.map(c => c.args)).includes(db.password));
  assert.ok(!calls.some(c => c.args.includes('prune')));
});

test('PostgreSQL cleanup preserves the volume if its container cannot be removed', async () => {
  const { PerformancePostgres } = await import('./performance-postgres.mjs');
  const calls = [];
  const db = new PerformancePostgres('test-image', async args => {
    calls.push(args);
    if (args[0] === 'ps') return 'owned-container';
    if (args[0] === 'rm') throw new Error('Docker rm failed');
    return '';
  });
  db.containerAttempted = true; db.volumeCreated = true;
  await assert.rejects(db.stop(), { message: 'Docker rm failed' });
  assert.ok(!calls.some(args => args[0] === 'volume'));
});

test('interrupted PostgreSQL runs remove their container and volume and preserve invalid evidence', { timeout: 240_000 }, async t => {
  if (process.env.PERFORMANCE_POSTGRES_TESTS !== '1') { t.skip('set PERFORMANCE_POSTGRES_TESTS=1 with Docker and a release server'); return; }
  const { spawn } = await import('node:child_process');
  const { once } = await import('node:events');
  const { readFile } = await import('node:fs/promises');
  const { docker } = await import('./performance-postgres.mjs');
  const directory = await mkdtemp(path.join(tmpdir(), 'waw-pg-cleanup-'));
  const output = path.join(directory, 'run');
  const terrain = await installTerrain(directory);
  const script = new URL('./performance-server.mjs', import.meta.url);
  const child = spawn(process.execPath, [script.pathname, '--storage', 'postgres', '--profile', 'regional',
    '--clients', '1', '--warmup', '0', '--duration', '60', '--output', output], { env: { ...process.env, TERRAIN_DIR: terrain }, stdio: ['ignore', 'pipe', 'pipe'] });
  const exited = once(child, 'exit');
  let text = '', started = false;
  child.stdout.on('data', chunk => { text += chunk; if (!started && text.includes('Performance regional:')) { started = true; child.kill('SIGTERM'); } });
  child.stderr.on('data', chunk => { text += chunk; });
  const timeout = setTimeout(() => child.kill('SIGTERM'), 200_000);
  try {
    const [code] = await exited;
    assert.equal(started, true, text); assert.equal(code, 1, text);
    const summary = JSON.parse(await readFile(path.join(output, 'summary.json'), 'utf8'));
    const metadata = JSON.parse(await readFile(path.join(output, 'metadata.json'), 'utf8'));
    assert.equal(summary.valid, false);
    assert.ok(summary.errors.includes('Run terminated'));
    assert.equal(await docker(['ps', '-aq', '--filter', `name=^/${metadata.postgres.container_name}$`]), '');
    await assert.rejects(docker(['volume', 'inspect', metadata.postgres.volume_name]));
    assert.ok(!JSON.stringify(metadata).includes('postgres://'));
  } finally { clearTimeout(timeout); child.kill('SIGTERM'); await rm(directory, { recursive: true, force: true }); }
});

test('performance clock preserves stream gaps and deadlines across wall-clock corrections', async () => {
  const { createClock } = await import('./performance-clock.mjs');
  let wall = 10_000, mono = 100;
  const clock = createClock(() => wall, () => mono);
  const before = clock.now(), deadline = before + 300_000;
  wall -= 1000; mono += 250;
  assert.equal(clock.now() - before, 250);
  assert.ok(clock.now() < deadline);
  wall += 60_000; mono += 299_750;
  assert.equal(clock.now(), deadline);
});

test('audit stages retain aggregate metrics and database diagnostic reset timestamps', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'waw-audit-stages-'));
  try {
    const server = [{ kind: 'trace_start', unix_ms: 1000, elapsed_ms: 0 },
      { kind: 'persistence_audit_prepare', success: true, commit_id: 4, rows: 7, role_rows: 3, bytes: 800, duration_ms: 2, elapsed_ms: 100 },
      { kind: 'persistence_audit_records', success: true, commit_id: 4, rows: 7, bytes: 800, duration_ms: 12, elapsed_ms: 120 },
      { kind: 'trace_end', written: 3, dropped: 0 }];
    const db = { kind: 'database_resource', time: 1150, checkpointer: { num_timed: 2, stats_reset: '2026-10-11' }, wal_stats: { wal_bytes: 900, stats_reset: '2026-10-11' }, io_stats: [{ writes: 8, stats_reset: '2026-10-11' }] };
    await writeFile(path.join(directory, 'server.jsonl'), server.map(JSON.stringify).join('\n'));
    await writeFile(path.join(directory, 'clients.jsonl'), JSON.stringify(db));
    const result = await analyze(directory, 1050, 1300);
    assert.equal(result.valid, true);
    assert.equal(result.metrics.server.persistence_audit_prepare_role_rows.max, 3);
    assert.equal(result.metrics.server.persistence_audit_records_duration_ms.max, 12);
    assert.equal(result.metrics.server.persistence_audit_records_bytes.max, 800);
    assert.deepEqual(result.database[0], db);
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('PostgreSQL samples include checkpoint, WAL and I/O counters without enabling timing or weakening durability', async () => {
  const { PerformancePostgres } = await import('./performance-postgres.mjs');
  let query;
  const db = new PerformancePostgres('test-image', async (args, options) => {
    if (args[0] === 'exec') { query = options.input; return JSON.stringify({ checkpointer: { stats_reset: 'epoch' }, wal_stats: { stats_reset: 'epoch' }, io_stats: [] }); }
    if (args[0] === 'stats') return '{}';
    assert.fail('unexpected Docker operation');
  });
  db.initialLsn = '0/123';
  const result = await db.sample();
  assert.equal(result.checkpointer.stats_reset, 'epoch');
  assert.ok(query.includes('pg_stat_checkpointer') && query.includes('pg_stat_wal') && query.includes('pg_stat_io') && query.includes('pg_stat_activity'));
  assert.ok(!/ALTER|SET /i.test(query));
});


test('frame rates include stalled buckets, fractional speeds and partial measurement seconds', async () => {
  const { FrameProgress, parseTimeScale } = await import('./performance-lib.mjs');
  const clock = new FrameProgress(1000, 3500);
  const sample = (frames = 1, scale = 0.25, id = 'a') => [{ game_id: id, frames, game_seconds: frames * scale / 60, whole_seconds: 0, requested_time_scale: scale }];
  clock.add(900, sample()); // warmup cannot contribute an interval or count
  clock.add(1000, sample()); clock.add(1020, sample());
  clock.add(2200, sample(0)); // maintenance cannot count as a committed frame
  clock.add(3100, sample()); clock.add(3200, sample(1, 60, 'b'));
  clock.add(3500, sample()); // exclusive end
  const result = clock.summary().a;
  assert.equal(result.committed_frames, 3);
  assert.equal(result.frame_hz, 3 / 2.5);
  assert.equal(result.achieved_time_scale, (3 * 0.25 / 60) / 2.5);
  assert.equal(result.interval_ms.samples, 2);
  assert.equal(result.interval_ms.max, 2080);
  assert.equal(result.series[1].frame_hz, 0);
  assert.equal(result.series[2].frame_hz, 2);
  assert.equal(clock.summary().b.achieved_time_scale, 1 / 2.5);
  for (const scale of [0.25, 1, 2, 5, 10, 30, 60]) assert.equal(parseTimeScale(String(scale)), scale);
  for (const invalid of ['0', '3', 'NaN', 'Infinity']) assert.throws(() => parseTimeScale(invalid));
  assert.throws(() => clock.add(3400, sample(-1)));
  assert.throws(() => clock.add(3400, [{ ...sample()[0], game_seconds: 9 }]));
});

test('version four analysis counts successful advancing cycles and correlates flush costs', async () => {
  const { validateRun } = await import('./performance-lib.mjs');
  const directory = await mkdtemp(path.join(tmpdir(), 'waw-frame-test-'));
  try {
    const progress = [{ game_id: 'a', frames: 1, game_seconds: 1 / 60, whole_seconds: 0, requested_time_scale: 1 }];
    const server = [
      { kind: 'trace_start', unix_ms: 1000, elapsed_ms: 0 },
      { kind: 'tick', game_id: 'a', frame: 1, duration_ms: 2, elapsed_ms: 10 },
      { kind: 'server_cycle', cycle_id: 1, success: true, running_games: ['a'], frame_progress: [], duration_ms: 900, elapsed_ms: 20 },
      { kind: 'persistence_flush', commit_id: 2, success: true, duration_ms: 50, elapsed_ms: 90 },
      { kind: 'persistence_commit', commit_id: 2, cycle_id: 2, success: true, duration_ms: 60, journal_bytes: 100, checkpoints: 1, elapsed_ms: 95 },
      { kind: 'server_cycle', cycle_id: 2, success: true, running_games: ['a'], frame_progress: progress, duration_ms: 70, elapsed_ms: 100 },
      { kind: 'server_cycle', cycle_id: 3, success: false, running_games: ['a'], frame_progress: progress, duration_ms: 80, elapsed_ms: 200 },
      { kind: 'trace_end', written: 7, dropped: 0 }
    ];
    await writeFile(path.join(directory, 'server.jsonl'), server.map(JSON.stringify).join('\n'));
    await writeFile(path.join(directory, 'clients.jsonl'), '');
    let result = await analyze(directory, 1000, 2000);
    assert.equal(result.frames.a.committed_frames, 1);
    assert.equal(result.committed_games.a, 1);
    assert.equal(result.metrics.server.cycle_duration_ms.max, 70);
    assert.equal(result.slowest_cycles[0].commit.stages.persistence_flush, 50);
    assert.equal(result.persistence.journal_bytes, 100);
    assert.equal(result.persistence.per_wall_second.journal_bytes, 100);
    assert.equal(result.valid, false);
    // Old successful-cycle telemetry alone cannot validate a new frame baseline.
    result = { valid: true, errors: [], committed_games: { a: 1 }, metrics: { server: { cycle_duration_ms: { samples: 1 } } }, frames: {} };
    validateRun(result, { measurement_version: 4, requested_time_scale: 1, games: [{ id: 'a' }], measurement_completed: true });
    assert.equal(result.valid, false);
    assert.ok(result.errors.includes('Missing committed frame telemetry: a'));
  } finally { await rm(directory, { recursive: true, force: true }); }
});


test('stream watchdog waits for expected game-second delivery while clock progress remains independent', async () => {
  const { streamStalled } = await import('./performance-lib.mjs');
  const player = { role: {}, lastFrame: 100, streamConnectedAt: 0, streamTick: 6 };
  // A slow durable clock can commit fractional frames for over fifteen wall seconds.
  assert.equal(streamStalled(player, 6, 20_000), false);
  // A resumed network connection may legitimately wait for a sequence newer
  // than after_sequence; retain the last delivered tick across reconnect.
  assert.equal(streamStalled({ role: {}, streamTick: 6, streamConnectedAt: 0 }, 6, 20_000), false);
  assert.equal(streamStalled(player, 7, 21_000), false);
  assert.equal(streamStalled(player, 8, 36_001), true);
  // Receiving the current game second clears the outstanding delivery deadline.
  player.streamTick = 8;
  assert.equal(streamStalled(player, 8, 36_002), false);
  assert.equal(player.streamBehindSince, undefined);
  // At high speeds a healthy stream can remain behind the latest HTTP tick;
  // satisfying each outstanding expectation must restart its delivery deadline.
  assert.equal(streamStalled(player, 100, 37_000), false);
  player.streamTick = 100;
  assert.equal(streamStalled(player, 200, 52_001), false);
  assert.equal(player.streamExpectedTick, 200);
  assert.equal(streamStalled({ role: { observer_kind: 'monitor' }, lastFrame: 100 }, 6, 20_000), true);
  assert.equal(streamStalled({ role: {}, streamConnectedAt: 0 }, 0, 15_001), true);
});


test('reconnect freshness uses fractional frames without requiring whole-second advancement', async () => {
  const { freshReconnectSnapshot } = await import('./performance-lib.mjs');
  assert.equal(freshReconnectSnapshot({ tick: 12, frame: 740 }, { tick: 12, frame: 745 }), true);
  assert.equal(freshReconnectSnapshot({ tick: 12, frame: 740 }, { tick: 12, frame: 740 }), false);
  assert.equal(freshReconnectSnapshot({ tick: 12, frame: 740 }, { tick: 11, frame: 745 }), false);
  assert.equal(freshReconnectSnapshot({ tick: 12 }, { tick: 12 }), true);
  assert.equal(freshReconnectSnapshot({ tick: 12 }, { tick: 11 }), false);
});

test('termination during player setup aborts requests and cleans up its server', { timeout: 30_000 }, async t => {
  const { spawn } = await import('node:child_process');
  const { once } = await import('node:events');
  const { access, readFile } = await import('node:fs/promises');
  const { fileURLToPath } = await import('node:url');
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
  try { await access(path.join(root, 'target/release/world-at-war-server')); }
  catch { t.skip('Build the release server to exercise setup cleanup'); return; }
  if (process.platform !== 'linux') { t.skip('Reference harness requires Linux'); return; }
  const directory = await mkdtemp(path.join(tmpdir(), 'waw-setup-cleanup-'));
  const output = path.join(directory, 'run');
  const child = spawn(process.execPath, [path.join(root, 'web/scripts/performance-server.mjs'),
    '--profile', 'global', '--clients', '200', '--warmup', '0', '--duration', '60', '--output', output],
  { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  const exited = once(child, 'exit');
  let text = ''; child.stdout.on('data', b => { text += b; }); child.stderr.on('data', b => { text += b; });
  const timeout = setTimeout(() => child.kill('SIGKILL'), 20_000);
  try {
    const deadline = performance.now() + 10_000;
    let setupStarted = false;
    while (performance.now() < deadline) {
      const log = await readFile(path.join(output, 'server.log'), 'utf8').catch(() => '');
      if (log.includes('guest_created')) { setupStarted = true; break; }
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    assert.equal(setupStarted, true, text);
    child.kill('SIGTERM');
    const [code] = await exited;
    assert.equal(code, 1, text);
    const metadata = JSON.parse(await readFile(path.join(output, 'metadata.json')));
    assert.equal(metadata.measurement_start_unix_ms, undefined, 'interruption must precede measurement');
    const summary = JSON.parse(await readFile(path.join(output, 'summary.json')));
    assert.equal(summary.valid, false);
    assert.ok(summary.errors.includes('Run terminated'));
    const trace = (await readFile(path.join(output, 'server.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse);
    assert.equal(trace.at(-1).kind, 'trace_end');
    assert.throws(() => process.kill(trace[0].pid, 0), { code: 'ESRCH' });
  } finally { clearTimeout(timeout); child.kill('SIGTERM'); await rm(directory, { recursive: true, force: true }); }
});


test('clock watchdog excludes planned host outages and resumes its deadline after recovery', async () => {
  const { clockStalled } = await import('./performance-lib.mjs');
  const host = { disconnected: true, lastClockProgress: 0 };
  assert.equal(clockStalled(host, 20_000), false);
  host.disconnected = false; host.lastClockProgress = 20_000;
  assert.equal(clockStalled(host, 20_000), false);
  assert.equal(clockStalled(host, 35_001), true);
});


test('a delayed reconnect handshake stays offline until its fresh clock deadline is ready', async () => {
  const { resumeConnection, clockStalled } = await import('./performance-lib.mjs');
  let open;
  const player = { disconnected: true, lastClockProgress: 0, connect: () => new Promise(resolve => { open = resolve; }) };
  const recovering = resumeConnection(player, () => 25_000);
  assert.equal(player.disconnected, true);
  assert.equal(clockStalled(player, 24_999), false);
  open(); await recovering;
  assert.equal(player.disconnected, false);
  assert.equal(player.lastClockProgress, 25_000);
  assert.equal(clockStalled(player, 25_000), false);
  const failed = { disconnected: true, lastClockProgress: 0, connect: async () => { throw new Error('handshake failed'); } };
  await assert.rejects(resumeConnection(failed, () => 25_000));
  assert.equal(failed.disconnected, true);
});
