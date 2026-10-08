import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { createWriteStream } from 'node:fs';
import { mkdir, mkdtemp, readFile, readdir, stat, writeFile, rm } from 'node:fs/promises';
import { once } from 'node:events';
import { finished } from 'node:stream/promises';
import { tmpdir, cpus, totalmem, release } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID, createHash } from 'node:crypto';
import { parseArgs } from 'node:util';
import WebSocket from 'ws';
import { analyze, profiles } from './performance-lib.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const { values } = parseArgs({ options: {
  profile: { type: 'string', default: 'mixed' }, clients: { type: 'string' },
  warmup: { type: 'string', default: '60' }, duration: { type: 'string', default: '300' },
  seed: { type: 'string', default: '12787750' }, output: { type: 'string' },
  'reconnect-every': { type: 'string', default: '300' },
  'check-budgets': { type: 'boolean', default: false },
} });
const numeric = (key, min, max) => {
  const n = Number(values[key]); assert.ok(Number.isSafeInteger(n) && n >= min && n <= max, `${key} must be ${min}..${max}`); return n;
};
const warmup = numeric('warmup', 0, 3600), duration = numeric('duration', 1, 86400);
const seed = numeric('seed', 0, Number.MAX_SAFE_INTEGER), reconnectEvery = numeric('reconnect-every', 15, 86400);
const layout = profiles(values.profile, values.clients === undefined ? undefined : Number(values.clients));
const output = path.resolve(values.output ?? path.join(root, 'tmp/performance', `${new Date().toISOString().replaceAll(':', '-')}-${values.profile}`));
await mkdir(path.dirname(output), { recursive: true });
await mkdir(output, { recursive: false });
const errors = []; const players = [], games = [], tasks = []; let stopping = false, backend, provider, base, serverLog;
let measurementStart, measurementEnd, workloadStart; let phase = 'setup'; let recording = true;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function emit(row) {
  if (!recording) return;
  if (sink.writableLength > 8 * 1024 * 1024) { stopping = true; throw new Error('Client trace writer backlog exceeded 8 MiB'); }
  sink.write(JSON.stringify({ time: Date.now(), phase, ...row }) + '\n');
}
function fail(error) { const message = String(error?.message ?? error); errors.push(message); emit({ kind: 'error', message }); stopping = true; }
process.on('SIGINT', () => { fail(new Error('Run interrupted')); });
process.on('SIGTERM', () => { fail(new Error('Run terminated')); });
const metadata = { schema_version: 1, profile: values.profile, layout, seed, warmup_seconds: warmup, measured_seconds: duration,
  reconnect_every_seconds: reconnectEvery, started_at: new Date().toISOString(),
  revision: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  dirty: execFileSync('git', ['status', '--porcelain'], { cwd: root, encoding: 'utf8' }).trim().length > 0,
  c3mesh_revision: (await readFile(path.join(root, 'c3mesh-revision.txt'), 'utf8')).trim(),
  rustc: execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim(), node: process.version,
  os: process.platform, os_release: release(), cpu: cpus()[0]?.model, logical_cpus: cpus().length, memory_bytes: totalmem(),
  binary_sha256: createHash('sha256').update(await readFile(path.join(root, 'target/release/world-at-war-server'))).digest('hex'),
  protocol: '1 Hz full snapshots; state/planning HTTP plus network WebSocket, no browser rendering; encoded HTTP body bytes when Content-Length is present, otherwise decoded bytes',
};
const runtime = await mkdtemp(path.join(tmpdir(), 'waw-performance-'));
const sink = createWriteStream(path.join(output, 'clients.jsonl'), { flags: 'wx' });
sink.on('error', error => { errors.push(`Client trace: ${error.message}`); stopping = true; });
await writeFile(path.join(output, 'metadata.json'), JSON.stringify(metadata, null, 2));

class Player {
  constructor(index) { this.index = index; this.cookie = ''; this.cookies = new Map(); this.csrf = ''; this.disconnected = false; this.controllers = new Set(); this.pending = []; this.lastProgress = Date.now(); }
  get prefix() { return `/v1/games/${this.game.id}`; }
  get query() { return new URLSearchParams({ role_id: this.role.id, lease_generation: String(this.role.lease_generation) }); }
  async request(resource, method = 'GET', body, expected = 200, operation = resource.replace(/[0-9a-f]{8}-[0-9a-f-]{27}/g, ':id').split('?')[0]) {
    const controller = new AbortController(); this.controllers.add(controller);
    const timeout = setTimeout(() => controller.abort(), 10_000); const started = performance.now();
    try {
      const response = await fetch(base + resource, { method, headers: { origin: base, cookie: this.cookie, 'x-csrf-token': this.csrf, 'content-type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body), signal: controller.signal });
      const bytes = await response.arrayBuffer();
      emit({ kind: 'http', client: this.index, game_id: this.game?.id, operation, status: response.status,
        duration_ms: performance.now() - started, bytes: bytes.byteLength,
        encoded_bytes: response.headers.has('content-length') ? Number(response.headers.get('content-length')) : undefined });
      assert.equal(response.status, expected, `${method} ${operation}: HTTP ${response.status}`);
      for (const cookie of response.headers.getSetCookie()) { const pair = cookie.split(';')[0]; const index = pair.indexOf('='); this.cookies.set(pair.slice(0, index), pair.slice(index + 1)); }
      this.cookie = [...this.cookies].map(([key, value]) => `${key}=${value}`).join('; ');
      const data = JSON.parse(Buffer.from(bytes).toString()); if (data.csrf_token) this.csrf = data.csrf_token;
      return data;
    } finally { clearTimeout(timeout); this.controllers.delete(controller); }
  }
  progress(tick, source = 'http') {
    this.progressBySource ??= new Map();
    if (!Number.isFinite(tick)) throw new Error(`Client ${this.index} received invalid tick`);
    if (this.progressBySource.has(source) && tick < this.progressBySource.get(source)) throw new Error(`Client ${this.index} tick regressed`);
    this.progressBySource.set(source, tick);
    if (this.lastTick === undefined || tick > this.lastTick) { this.lastTick = tick; this.lastProgress = Date.now(); }
  }
  async snapshot() {
    const data = await this.request(`${this.prefix}/${this.role.observer_kind ? 'truth' : 'state'}?${this.query}`);
    this.progress(data.tick); return data;
  }
  async connect() {
    const endpoint = this.role.observer_kind ? 'truth/stream' : 'network/stream';
    const query = this.query; if (this.sequence !== undefined && !this.role.observer_kind) query.set('after_sequence', this.sequence);
    const socket = new WebSocket(`${base.replace('http:', 'ws:')}${this.prefix}/${endpoint}?${query}`, { headers: { cookie: this.cookie, origin: base }, handshakeTimeout: 10_000 });
    this.socket = socket; this.lastFrame = undefined; this.streamConnectedAt = Date.now();
    socket.on('message', bytes => {
      try {
        const now = Date.now(); const frame = JSON.parse(String(bytes));
        this.progress(this.role.observer_kind ? frame.tick : frame.projection.tick, 'websocket');
        if (!this.role.observer_kind) {
          if (!frame.resync && this.sequence !== undefined && frame.sequence <= this.sequence) throw new Error('Non-monotonic stream sequence');
          this.sequence = frame.sequence;
        }
        emit({ kind: 'websocket', client: this.index, game_id: this.game.id, bytes: bytes.length,
          gap_ms: this.lastFrame === undefined ? undefined : now - this.lastFrame }); this.lastFrame = now;
      } catch (error) { fail(error); }
    });
    socket.on('error', error => { if (!stopping && !this.disconnected) fail(new Error(`Client ${this.index} WebSocket: ${error.message}`)); });
    socket.on('close', code => { if (!stopping && !this.disconnected) fail(new Error(`Client ${this.index} unexpected WebSocket closure ${code}`)); });
    await once(socket, 'open');
  }
  loop(interval, operation) {
    const task = (async () => {
      while (!stopping) {
        if (!this.disconnected) {
          try { await operation(); } catch (error) { if (!this.disconnected && !stopping) fail(error); }
        }
        // Small chunks allow prompt teardown without leaving renewal timers alive.
        const until = Date.now() + interval;
        while (!stopping && Date.now() < until) await sleep(Math.min(100, until - Date.now()));
      }
    })(); tasks.push(task);
  }
  async begin() {
    await this.connect();
    this.loop(30_000, async () => { this.role = await this.request(`${this.prefix}/roles/${this.role.id}/renew`, 'POST', { lease_generation: this.role.lease_generation }); });
    if (!this.role.observer_kind) {
      this.loop(1000, () => this.snapshot());
      this.loop(1000, () => this.request(`${this.prefix}/planning?${this.query}`));
      this.loop(2000, async () => {
        await this.request(`${this.prefix}/authority`);
        await this.request(`${this.prefix}/authority/requests?${this.query}`);
      });
    }
    if (this.commander) this.loop(30_000, async () => {
      const intent = { intent_id: randomUUID(), issuer_role: this.role.id, target: this.role.command_units[0],
        kind: { Move: { north_mps: 0, east_mps: 0 } }, requested_tick: this.lastTick ?? 0 };
      const result = await this.request(`${this.prefix}/roles/${this.role.id}/intent`, 'POST', { lease_generation: this.role.lease_generation, intent });
      this.pending.push(intent.intent_id); emit({ kind: 'intent', game_id: this.game.id, state: result.status });
    });
    if (this.commander) this.loop(2000, async () => {
      for (const id of [...this.pending]) {
        const receipt = await this.request(`${this.prefix}/roles/${this.role.id}/intents/${id}?${this.query}`);
        emit({ kind: 'intent', game_id: this.game.id, state: receipt.state });
        if (['executed', 'dropped', 'expired', 'rejected', 'denied'].includes(receipt.state)) this.pending.splice(this.pending.indexOf(id), 1);
      }
    });
  }
  async reconnect() {
    this.disconnected = true; this.socket?.terminate(); for (const c of this.controllers) c.abort();
    await sleep(10_000); if (stopping) return;
    const started = performance.now(); const before = this.lastTick;
    this.role = await this.request(`${this.prefix}/roles/${this.role.id}/resume`, 'POST', {});
    const snapshot = await this.snapshot(); assert.ok(snapshot.tick > before, 'Reconnect did not receive a fresh snapshot');
    this.disconnected = false; await this.connect(); this.lastProgress = Date.now();
    emit({ kind: 'reconnect', client: this.index, game_id: this.game.id, duration_ms: performance.now() - started });
  }
}
async function stopBackend() {
  if (!backend || backend.exitCode !== null || backend.signalCode !== null) return;
  const exited = once(backend, 'exit'); backend.kill('SIGTERM');
  const timer = setTimeout(() => backend.kill('SIGKILL'), 10_000);
  try { await exited; } finally { clearTimeout(timer); }
}
async function resources() {
  const status = await readFile(`/proc/${backend.pid}/status`, 'utf8');
  const fields = (await readFile(`/proc/${backend.pid}/stat`, 'utf8')).split(') ')[1].split(' ');
  let auditBytes = 0; for (const name of await readdir(path.join(runtime, 'events'))) auditBytes += (await stat(path.join(runtime, 'events', name))).size;
  emit({ kind: 'resource', server_rss_bytes: Number(status.match(/VmRSS:\s+(\d+)/)[1]) * 1024,
    server_cpu_seconds: (Number(fields[11]) + Number(fields[12])) / metadata.clock_ticks_per_second,
    audit_bytes: auditBytes, generator_rss_bytes: process.memoryUsage().rss, generator_cpu: process.cpuUsage() });
}
try {
  assert.equal(process.platform, 'linux', 'Reference harness requires Linux/WSL process telemetry');
  metadata.clock_ticks_per_second = Number(execFileSync('getconf', ['CLK_TCK'], { encoding: 'utf8' }).trim());
  provider = createServer((request, response) => {
    request.resume(); response.setHeader('content-type', 'application/json');
    if (request.url === '/login') { response.setHeader('set-cookie', 'fixture=1'); response.end('{"Login":"Success"}'); }
    else if (request.url === '/gp') response.end(JSON.stringify([{ NORAD_CAT_ID: '25544', OBJECT_NAME: 'ISS (ZARYA)', OBJECT_TYPE: 'PAYLOAD' }, { NORAD_CAT_ID: '5', OBJECT_NAME: 'VANGUARD 1', OBJECT_TYPE: 'PAYLOAD' }]));
    else { response.statusCode = 404; response.end('{}'); }
  });
  provider.listen(0, '127.0.0.1'); await once(provider, 'listening');
  const providerBase = `http://127.0.0.1:${provider.address().port}`;
  const portServer = createServer(); portServer.listen(0, '127.0.0.1'); await once(portServer, 'listening');
  const port = portServer.address().port; await new Promise(resolve => portServer.close(resolve)); base = `http://127.0.0.1:${port}`;
  await mkdir(path.join(runtime, 'airports'));
  const airports = [JSON.parse(await readFile(path.join(root, 'web/e2e/fixtures/airport.json'), 'utf8'))];
  await writeFile(path.join(runtime, 'airports/latest.json'), JSON.stringify({ schema_version: 1, synced_unix: Math.floor(Date.now() / 1000), checksum: 'performance-fixture', sources: [], degraded_sources: [], airports }));
  serverLog = createWriteStream(path.join(output, 'server.log'), { flags: 'wx' });
  backend = spawn(path.join(root, 'target/release/world-at-war-server'), [], { cwd: runtime,
    env: { PATH: process.env.PATH, BIND_ADDR: `127.0.0.1:${port}`, APP_ALLOWED_ORIGINS: base,
      COOKIE_SECURE: 'false', ADMIN_SETUP_TOKEN: '', PERFORMANCE_TRACE_DIR: output,
      SPACETRACK_USERNAME: '', SPACETRACK_PASSWORD: '', SPACETRACK_LOGIN_URL: `${providerBase}/login`, SPACETRACK_GP_URL: `${providerBase}/gp`,
      AIRPORT_CACHE_DIR: path.join(runtime, 'airports'), AIRPORT_REFRESH_MAX_AGE_SECONDS: '86400',
      OURAIRPORTS_AIRPORTS_URL: `${providerBase}/airports`, OURAIRPORTS_RUNWAYS_URL: `${providerBase}/runways`,
      FAA_NASR_SUBSCRIPTION_URL: `${providerBase}/faa`, FAA_NASR_APT_URL: '',
      SPACE_CARDS_DIR: path.join(runtime, 'cards'), NETWORK_EVENT_DIR: path.join(runtime, 'events'),
      COMMUNICATIONS_CATALOG_PATH: path.join(root, 'data/communications/catalog.yaml') }, stdio: ['ignore', 'pipe', 'pipe'] });
  backend.stdout.pipe(serverLog, { end: false }); backend.stderr.pipe(serverLog, { end: false });
  backend.on('error', fail); backend.on('exit', (code, signal) => { if (!stopping) fail(new Error(`Server exited: ${code ?? signal}`)); });
  const deadline = Date.now() + 30_000;
  while (!(await fetch(base + '/health').then(r => r.ok, () => false))) { assert.ok(!stopping && Date.now() < deadline, 'Server startup failed'); await sleep(100); }
  for (const [scenario, count] of layout) {
    const host = new Player(players.length); players.push(host);
    const session = await host.request('/v1/auth/guest', 'POST', { display_name: `Performance ${host.index}` }); host.playerId = session.player_id;
    if (scenario === 'global-crisis.v2' && !metadata.catalog) metadata.catalog = await host.request('/v1/admin/space-track/connect', 'POST', { username: 'fixture-user', password: 'fixture-password', remember: false });
    const { game } = await host.request('/v1/games', 'POST', { scenario_id: scenario, title: `Performance ${games.length}`, seed });
    host.game = game; games.push({ ...game, host });
    const roles = await host.request(host.prefix + '/roles');
    const selected = roles.filter(r => r.claimable || r.observer_kind).sort((a, b) => {
      const preferred = scenario === 'global-crisis.v2' ? '00000000-0000-0000-0000-00000000006f' : '00000000-0000-0000-0000-0000000000d3';
      return Number(b.id === preferred) - Number(a.id === preferred) || a.id.localeCompare(b.id);
    }).slice(0, count);
    assert.equal(selected.length, count, 'Insufficient scenario seats');
    for (let i = 0; i < selected.length; i++) {
      const player = i === 0 ? host : new Player(players.length);
      if (i > 0) { players.push(player); const session = await player.request('/v1/auth/guest', 'POST', { display_name: `Performance ${player.index}` }); player.playerId = session.player_id; player.game = game; await player.request(player.prefix + '/join', 'POST', { display_name: `Performance ${player.index}` }); }
      const role = selected[i];
      if (role.observer_kind) await host.request(`${host.prefix}/roles/${role.id}/observer-grant`, 'PUT', { target_player_id: player.playerId });
      player.role = await player.request(`${player.prefix}/roles/${role.id}/claim`, 'POST', {}); player.commander = i === 0;
      await player.snapshot();
      // A different player's live seat and a stale generation must remain inaccessible.
      if (i > 0) await player.request(`${player.prefix}/state?role_id=${host.role.id}&lease_generation=${host.role.lease_generation}`, 'GET', undefined, 403);
      await player.request(`${player.prefix}/${role.observer_kind ? 'truth' : 'state'}?role_id=${role.id}&lease_generation=${player.role.lease_generation + 1}`, 'GET', undefined, 403);
    }
    await host.request(host.prefix + '/start', 'POST', {});
  }
  metadata.games = games.map(({ host, ...game }) => game); metadata.clients = players.map(p => ({ index: p.index, game_id: p.game.id, role_id: p.role.id, observer_kind: p.role.observer_kind }));
  // Do not retain provider response fields such as remembered usernames in artifacts.
  metadata.catalog = metadata.catalog ? { checksum: metadata.catalog.checksum, object_count: metadata.catalog.object_count } : null;
  for (const player of players) { await player.begin(); await sleep(20); }
  workloadStart = Date.now(); measurementStart = workloadStart + warmup * 1000; measurementEnd = measurementStart + duration * 1000;
  metadata.measurement_start_unix_ms = measurementStart; metadata.measurement_end_unix_ms = measurementEnd;
  await writeFile(path.join(output, 'metadata.json'), JSON.stringify(metadata, null, 2));
  console.log(`Performance ${values.profile}: ${players.length} players / ${games.length} games; ${warmup}s warmup + ${duration}s measurement. Artifacts: ${output}`);
  let nextResource = 0, nextReconnect = measurementStart + reconnectEvery * 1000, cycle = 0, nextProgress = 0;
  while (!stopping && Date.now() < measurementEnd) {
    phase = Date.now() < measurementStart ? 'warmup' : 'measurement';
    const expected = Date.now() + 1000; await sleep(1000);
    emit({ kind: 'generator_lag', duration_ms: Math.max(0, Date.now() - expected) });
    for (const p of players) if (!p.disconnected && (Date.now() - p.lastProgress > 15_000 || Date.now() - (p.lastFrame ?? p.streamConnectedAt) > 15_000)) fail(new Error(`Game stalled for client ${p.index}`));
    if (Date.now() >= nextResource) { await resources(); nextResource = Date.now() + 5000; }
    if (Date.now() >= nextReconnect && Date.now() + 15_000 < measurementEnd) {
      const count = Math.max(1, Math.ceil(players.length / 10));
      const selected = Array.from({ length: count }, (_, i) => players[(cycle * count + i) % players.length]); cycle++;
      tasks.push(Promise.all(selected.map(p => p.reconnect())).catch(fail)); nextReconnect += reconnectEvery * 1000;
    }
    if (Date.now() >= nextProgress) { console.log(`${phase}: ${Math.max(0, Math.floor((Date.now() - measurementStart) / 1000))}/${duration}s, errors=${errors.length}`); nextProgress = Date.now() + 60_000; }
  }
} catch (error) { fail(error); }
finally {
  stopping = true; phase = 'cleanup';
  for (const p of players) { p.disconnected = true; p.socket?.terminate(); for (const c of p.controllers) c.abort(); }
  await Promise.allSettled(tasks);
  for (const game of games) try { await game.host.request(game.host.prefix + '/pause', 'POST', {}); } catch (error) { fail(error); }
  await stopBackend();
  if (provider?.listening) await new Promise(resolve => provider.close(resolve));
  if (serverLog) { serverLog.end(); await finished(serverLog); }
  recording = false; sink.end(); await finished(sink);
  try {
    const result = await analyze(output, measurementStart, measurementEnd);
    result.valid &&= errors.length === 0 && Date.now() >= measurementEnd;
    result.errors = [...new Set([...result.errors, ...errors])];
    result.budgets = { regional: games.filter(g => g.scenario_id === 'regional-campaign.v1').map(g => {
      const metric = result.metrics[`game:${g.id}`]?.tick_duration_ms;
      return { game_id: g.id, p95_ms: metric?.p95, max_ms: metric?.max, met: !!metric && metric.p95 <= 250 && metric.max <= 1000 };
    }), reconnect: Object.entries(result.metrics).filter(([key, m]) => key.startsWith('client:') && m.reconnect_ms).map(([client, m]) => ({ client, max_ms: m.reconnect_ms.max, met: m.reconnect_ms.max <= 3000 })) };
    await writeFile(path.join(output, 'summary.json'), JSON.stringify(result, null, 2));
    console.log(JSON.stringify({ valid: result.valid, errors: result.errors, budgets: result.budgets, output }, null, 2));
    if (!result.valid || (values['check-budgets'] && [...result.budgets.regional, ...result.budgets.reconnect].some(b => !b.met))) process.exitCode = 1;
  } catch (error) { console.error(error); process.exitCode = 1; }
  await rm(runtime, { recursive: true, force: true });
}
