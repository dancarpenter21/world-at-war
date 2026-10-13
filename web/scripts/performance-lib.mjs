import { createReadStream } from 'node:fs';
import { createInterface } from 'node:readline';

// Bounded 0.1 ms histogram up to 10 seconds; slower samples remain in an overflow bin.
export class Metric {
  constructor(resolution = 0.1, ceiling = 10_000) {
    this.resolution = resolution; this.ceiling = ceiling; this.bins = new Map();
    this.count = 0; this.sum = 0; this.max = null;
  }
  add(value) {
    if (!Number.isFinite(value) || value < 0) throw new Error(`Invalid metric: ${value}`);
    this.count++; this.sum += value; this.max = Math.max(this.max ?? 0, value);
    const bin = Math.min(Math.ceil(value / this.resolution), Math.ceil(this.ceiling / this.resolution) + 1);
    this.bins.set(bin, (this.bins.get(bin) ?? 0) + 1);
  }
  summary() {
    const entries = [...this.bins].sort((a, b) => a[0] - b[0]);
    const percentile = p => {
      if (!this.count) return null;
      const rank = Math.ceil(this.count * p); let count = 0;
      for (const [bin, n] of entries) {
        count += n;
        if (count >= rank) return bin * this.resolution > this.ceiling ? this.max : bin * this.resolution;
      }
    };
    return { samples: this.count, mean: this.count ? this.sum / this.count : null,
      p50: percentile(0.5), p95: percentile(0.95), max: this.max,
      histogram_resolution: this.resolution, histogram_ceiling: this.ceiling };
  }
}
export function parseTimeScale(value) {
  const scale = Number(value);
  if (![0.25, 1, 2, 5, 10, 30, 60].includes(scale)) throw new Error('time-scale must be 0.25, 1, 2, 5, 10, 30 or 60');
  return scale;
}
// Network snapshots publish when their game-second sequence changes; observer
// streams publish each wall second. Quiet fractional frames are not a stalled socket.
export function streamStalled(player, expectedTick, now) {
  if (player.streamTick === undefined || player.role.observer_kind) {
    return now - (player.lastFrame ?? player.streamConnectedAt) > 15_000;
  }
  if (player.streamExpectedTick !== undefined && player.streamTick >= player.streamExpectedTick) {
    player.streamExpectedTick = undefined; player.streamBehindSince = undefined;
  }
  if (expectedTick !== undefined && expectedTick > player.streamTick) {
    player.streamExpectedTick ??= expectedTick;
    player.streamBehindSince ??= now;
    return now - player.streamBehindSince > 15_000;
  }
  player.streamExpectedTick = undefined; player.streamBehindSince = undefined;
  return false;
}
export async function resumeConnection(player, now) {
  await player.connect();
  player.lastProgress = now();
  player.lastClockProgress = player.lastProgress;
  player.disconnected = false;
}
export function clockStalled(host, now) {
  return !host.disconnected && now - host.lastClockProgress > 15_000;
}
export function freshReconnectSnapshot(before, snapshot) {
  if (!Number.isFinite(snapshot.tick) || snapshot.tick < before.tick) return false;
  if (Number.isSafeInteger(before.frame) && Number.isSafeInteger(snapshot.frame)) return snapshot.frame > before.frame;
  // Truth and historical projections may omit the frame. A current authenticated
  // HTTP response need not advance the whole-second tick; the host clock watchdog
  // independently verifies committed progress.
  return true;
}
export function profiles(name, clients) {
  const definitions = { regional: [['regional-campaign.v1', 11]], global: [['global-crisis.v2', 13]],
    mixed: [['regional-campaign.v1', 11], ['regional-campaign.v1', 11], ['regional-campaign.v1', 11], ['global-crisis.v2', 13], ['global-crisis.v2', 4]] };
  if (!definitions[name]) throw new Error(`Unknown profile: ${name}`);
  if (clients === undefined) return definitions[name];
  if (!Number.isInteger(clients) || clients < 1 || clients > 200) throw new Error('clients must be 1..200');
  const result = []; let remaining = clients, index = 0;
  while (remaining) {
    const [scenario, capacity] = definitions[name][index++ % definitions[name].length];
    const seats = Math.min(remaining, capacity); result.push([scenario, seats]); remaining -= seats;
  }
  return result;
}
export async function* records(file) {
  const lines = createInterface({ input: createReadStream(file), crlfDelay: Infinity });
  for await (const line of lines) if (line.trim()) yield JSON.parse(line);
}
// Counts include zero-progress wall-time buckets; legacy traces have no frame samples.
export class FrameProgress {
  constructor(start, end) {
    if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) throw new Error('Invalid measurement window');
    this.start = start; this.end = end; this.games = new Map();
  }
  add(time, progress) {
    for (const row of progress) {
      if (typeof row.game_id !== 'string' || !Number.isSafeInteger(row.frames) || row.frames < 0
        || !Number.isFinite(row.game_seconds) || row.game_seconds < 0
        || !Number.isSafeInteger(row.whole_seconds) || row.whole_seconds < 0
        || ![0.25, 1, 2, 5, 10, 30, 60].includes(row.requested_time_scale)
        || Math.abs(row.game_seconds - row.frames * row.requested_time_scale / 60) > 1e-7) throw new Error('Invalid committed frame progress');
      const game = this.games.get(row.game_id) ?? { frames: 0, seconds: 0, whole_seconds: 0, intervals: new Metric(), overruns: 0, buckets: new Map(), last: null, scales: new Set() };
      this.games.set(row.game_id, game);
      if (!row.frames) continue;
      if (game.last !== null && time < game.last) throw new Error('Committed frame time regressed');
      if (time >= this.start && time < this.end) {
        game.frames += row.frames; game.seconds += row.game_seconds; game.whole_seconds += row.whole_seconds;
        game.scales.add(row.requested_time_scale);
        if (game.last !== null && game.last >= this.start) {
          const interval = time - game.last; game.intervals.add(interval);
          if (interval > 1000 / 60) game.overruns++;
        }
        const index = Math.floor((time - this.start) / 1000);
        const bucket = game.buckets.get(index) ?? { frames: 0, game_seconds: 0 };
        bucket.frames += row.frames; bucket.game_seconds += row.game_seconds; game.buckets.set(index, bucket);
      }
      game.last = time;
    }
  }
  summary() {
    const seconds = (this.end - this.start) / 1000;
    return Object.fromEntries([...this.games].map(([id, game]) => {
      const scales = [...game.scales]; const requested = scales.length === 1 ? scales[0] : null;
      const series = Array.from({ length: Math.ceil(seconds) }, (_, index) => {
        const bucket = game.buckets.get(index) ?? { frames: 0, game_seconds: 0 };
        const duration = Math.min(1, seconds - index);
        return { elapsed_seconds: index, duration_seconds: duration, ...bucket, frame_hz: bucket.frames / duration, achieved_time_scale: bucket.game_seconds / duration };
      });
      return [id, { target_frame_hz: 60, committed_frames: game.frames, committed_game_seconds: game.seconds,
        whole_seconds: game.whole_seconds, frame_hz: game.frames / seconds, achieved_time_scale: game.seconds / seconds,
        requested_time_scale: requested, requested_scales: scales, requested_speed_percent: requested === null ? null : game.seconds / seconds / requested * 100,
        interval_ms: game.intervals.summary(), intervals_over_frame_period: game.overruns, series }];
    }));
  }
}

export async function analyze(directory, start, end) {
  const groups = new Map(); const errors = []; let traceEnd, serverEpoch, written = 0;
  const resources = []; const database = []; const committedGames = {}; const retention = {}; const outcomes = {};
  const frames = new FrameProgress(start, end);
  const persistence = { commits: 0, boundaries: 0, checkpoints: 0, journal_bytes: 0, snapshot_submitted_bytes: 0 };
  const stages = new Map(), cycleCommits = new Map(), slowestCycles = [];
  const audit = { events: 0, bytes: 0, cleanup: [] }; const traffic = new Map();
  const add = (group, name, value) => {
    if (value === undefined || value === null) return;
    if (!groups.has(group)) groups.set(group, new Map());
    const metrics = groups.get(group);
    if (!metrics.has(name)) metrics.set(name, new Metric(name.includes('bytes') ? 1 : 0.1, name.includes('bytes') ? 10_000_000 : 10_000));
    metrics.get(name).add(value);
  };
  for await (const row of records(`${directory}/server.jsonl`)) {
    if (row.kind === 'trace_end') { traceEnd = row; continue; }
    written++;
    if (row.kind === 'trace_start') serverEpoch = row.unix_ms - row.elapsed_ms;
    const time = serverEpoch + row.elapsed_ms;
    if (row.success === false && ['server_cycle', 'persistence_commit', 'mutation_boundary'].includes(row.kind)) errors.push(`Failed server operation: ${row.kind}`);
    if (row.kind === 'server_cycle' && row.success && row.frame_progress) frames.add(time, row.frame_progress);
    if (row.commit_id && row.kind.startsWith('persistence_')) {
      const span = stages.get(row.commit_id) ?? {}; stages.set(row.commit_id, span);
      if (row.success) span[row.kind] = row.duration_ms;
      if (row.kind === 'persistence_commit') {
        if (row.cycle_id != null) cycleCommits.set(row.cycle_id, { commit_id: row.commit_id, success: row.success, stages: span, checkpoints: row.checkpoints, journal_bytes: row.journal_bytes });
        stages.delete(row.commit_id);
      }
    }
    const committed = row.kind === 'server_cycle' ? cycleCommits.get(row.cycle_id) : undefined;
    if (row.kind === 'server_cycle') cycleCommits.delete(row.cycle_id);
    if (!(time >= start && time < end)) continue;
    const group = row.game_id ? `game:${row.game_id}` : 'server';
    if (row.kind === 'retention') {
      const entry = retention[group] ??= { samples: 0, first: null, last: null, peak: {}, series: [] };
      const counts = {};
      const flatten = (value, prefix = '') => {
        for (const [key, item] of Object.entries(value)) {
          const name = prefix ? `${prefix}.${key}` : key;
          if (item !== null && typeof item === 'object') flatten(item, name);
          else if (Number.isSafeInteger(item) && item >= 0) counts[name] = item;
          else errors.push(`Invalid retention counter: ${name}`);
        }
      };
      flatten({ simulation: row.simulation, server: row.server });
      const sample = { time, tick: row.tick, radio_tick: row.radio_tick, counts };
      entry.samples++; entry.first ??= sample; entry.last = sample; entry.series.push(sample);
      for (const [key, value] of Object.entries(counts)) entry.peak[key] = Math.max(entry.peak[key] ?? 0, value);
    }
    if (row.kind === 'server_cycle' && row.success) {
      // Idle cycles must not dilute the workload's percentiles.
      const advancing = row.frame_progress ? row.frame_progress.filter(p => p.frames > 0).map(p => p.game_id) : row.running_games;
      if (advancing?.length) {
        for (const id of advancing) committedGames[id] = (committedGames[id] ?? 0) + 1;
        slowestCycles.push({ time, cycle_id: row.cycle_id, duration_ms: row.duration_ms, scheduled_to_committed_ms: row.scheduled_to_committed_ms, games: advancing, commit: committed });
        slowestCycles.sort((a, b) => b.duration_ms - a.duration_ms); slowestCycles.length = Math.min(slowestCycles.length, 20);
        for (const key of ['duration_ms', 'schedule_delay_ms', 'scheduled_to_committed_ms']) add('server', `cycle_${key}`, row[key]);
      }
    }
    if (row.kind.startsWith('persistence_') || ['mutation_boundary', 'durability_gate'].includes(row.kind)) {
      if (row.success) {
        const stage = `${row.kind}${row.source ? `:${row.source}` : ''}`;
        for (const key of ['duration_ms', 'bytes', 'boundaries', 'checkpoints', 'journal_bytes', 'snapshot_submitted_bytes', 'archived_events', 'audit_events', 'audit_bytes', 'rows', 'role_rows', 'role_bytes', 'runtime_bytes', 'runtime_rows']) {
          add('server', `${stage}_${key}`, row[key]);
          if (row.game_id) add(group, `${stage}_${key}`, row[key]);
        }
      }
    }
    if (row.kind === 'persistence_commit' && row.success) {
      persistence.commits++;
      for (const key of ['boundaries', 'checkpoints', 'journal_bytes', 'snapshot_submitted_bytes']) persistence[key] += row[key] ?? 0;
      audit.events += row.audit_events ?? 0; audit.bytes += row.audit_bytes ?? 0; }
    if (row.kind === 'audit_cleanup') add('server', 'audit_cleanup_duration_ms', row.duration_ms);
    if (row.kind === 'audit_cleanup_result') audit.cleanup.push({ time, deleted_rows: row.deleted_rows ?? 0, failed: row.failed, eligible_backlog_rows: row.eligible_backlog_rows, oldest_eligible_age_seconds: row.oldest_eligible_age_seconds });
    if (row.kind === 'tick') {
      for (const name of ['duration_ms', 'schedule_delay_ms']) add(group, `tick_${name}`, row[name]);
      if (row.whole_seconds !== undefined) add(group, `${row.whole_seconds > 0 ? 'boundary' : 'fractional'}_processing_ms`, row.duration_ms);
    }
    if (row.kind === 'game_lock') for (const name of ['wait_ms', 'hold_ms']) {
      add('server', `lock_${name}`, row[name]); add(`lock:${row.caller}:${row.mode}`, name, row[name]);
    }
    if (row.kind === 'projection') for (const name of ['build_ms', 'build_including_lock_ms', 'serialization_ms', 'bytes']) {
      add(group, `${row.transport}_${name}`, row[name]);
    }
  }
  if (!Number.isFinite(serverEpoch) || !traceEnd || traceEnd.dropped !== 0 || traceEnd.written !== written) errors.push('Incomplete server trace or dropped samples');
  for await (const row of records(`${directory}/clients.jsonl`)) {
    if (row.kind === 'error') errors.push(row.message);
    if (row.time < start || row.time >= end) continue;
    if (row.kind === 'resource') resources.push(row);
    if (row.kind === 'database_resource') database.push(row);
    if (row.kind === 'generator_lag') add('generator', 'schedule_lag_ms', row.duration_ms);
    if (row.kind === 'intent') outcomes[row.state] = (outcomes[row.state] ?? 0) + 1;
    if (row.kind === 'http') {
      for (const group of [`game:${row.game_id}`, `client:${row.client}`, `http:${row.operation}`]) {
        add(group, 'request_ms', row.duration_ms); add(group, 'decoded_http_bytes', row.bytes);
        add(group, 'encoded_http_bytes', row.encoded_bytes);
      }
    }
    if (row.kind === 'reconnect') add(`client:${row.client}`, 'reconnect_ms', row.duration_ms);
    if (row.kind === 'websocket') add(`client:${row.client}`, 'stream_gap_ms', row.gap_ms);
    if (['http', 'websocket'].includes(row.kind)) {
      const key = `${row.client}:${Math.floor((row.time - start) / 1000)}`;
      traffic.set(key, (traffic.get(key) ?? 0) + (row.encoded_bytes ?? row.bytes));
    }
  }
  for (const [key, bytes] of traffic) add(`client:${key.split(':')[0]}`, 'received_bytes_per_active_second', bytes);
  const frameSummary = frames.summary();
  const duration = (end - start) / 1000;
  persistence.per_wall_second = Object.fromEntries(Object.entries(persistence).map(([key, value]) => [key, value / duration]));
  const gameSeconds = Object.values(frameSummary).reduce((sum, game) => sum + game.committed_game_seconds, 0);
  persistence.per_game_second = gameSeconds > 0 ? Object.fromEntries(Object.entries(persistence).filter(([, value]) => typeof value === 'number').map(([key, value]) => [key, value / gameSeconds])) : null;
  const growth = database.length >= 2 ? { sampled_seconds: (database.at(-1).time - database[0].time) / 1000,
    database_bytes: database.at(-1).database_bytes - database[0].database_bytes, wal_bytes: database.at(-1).wal_bytes - database[0].wal_bytes } : null;
  return { frames: frameSummary, persistence, database_growth: growth, slowest_cycles: slowestCycles, valid: errors.length === 0, errors, trace: traceEnd, measured_seconds: (end - start) / 1000, intent_observations: outcomes, resources, database, committed_games: committedGames, retention, audit,
    metrics: Object.fromEntries([...groups].map(([group, metrics]) => [group, Object.fromEntries([...metrics].map(([name, metric]) => [name, metric.summary()]))])) };
}

// Called only for new harness runs. Historical trace analysis stays compatible.
export function validateRun(result, metadata) {
  const reject = message => { result.errors.push(message); result.valid = false; };
  if (!metadata.measurement_completed) reject('Measurement window did not complete');
  for (const game of metadata.games ?? []) {
    if (!(result.committed_games[game.id] > 0)) reject(`Missing committed progress: ${game.id}`);
    if (metadata.measurement_version >= 4) {
      const progress = result.frames?.[game.id];
      if (!(progress?.committed_frames > 0)) reject(`Missing committed frame telemetry: ${game.id}`);
      if (progress?.requested_time_scale !== metadata.requested_time_scale) reject(`Unexpected measured game speed: ${game.id}`);
    }
  }
  if (!metadata.games?.length) reject('No games were measured');
  if (!result.metrics.server?.cycle_duration_ms?.samples) reject('Missing full server-cycle telemetry');
  if (metadata.storage === 'postgres') {
    if (!result.metrics.server?.persistence_commit_duration_ms?.samples) reject('Missing PostgreSQL commit telemetry');
    if (!result.database.length) reject('Missing PostgreSQL resource telemetry');
  }
  result.measurement_version = metadata.measurement_version;
  result.storage = metadata.storage;
}
