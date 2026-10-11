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
export async function analyze(directory, start, end) {
  const groups = new Map(); const errors = []; let traceEnd, serverEpoch, written = 0;
  const resources = []; const database = []; const committedGames = {}; const retention = {}; const outcomes = {}; const traffic = new Map();
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
    if (!(time >= start && time <= end)) continue;
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
      if (row.running_games?.length) {
        for (const id of row.running_games) committedGames[id] = (committedGames[id] ?? 0) + 1;
        for (const key of ['duration_ms', 'schedule_delay_ms', 'scheduled_to_committed_ms']) add('server', `cycle_${key}`, row[key]);
      }
    }
    if (row.kind.startsWith('persistence_') || ['mutation_boundary', 'durability_gate'].includes(row.kind)) {
      if (row.success) {
        const stage = `${row.kind}${row.source ? `:${row.source}` : ''}`;
        for (const key of ['duration_ms', 'bytes', 'boundaries', 'checkpoints', 'journal_bytes', 'snapshot_submitted_bytes', 'archived_events']) {
          add('server', `${stage}_${key}`, row[key]);
          if (row.game_id) add(group, `${stage}_${key}`, row[key]);
        }
      }
    }
    if (row.kind === 'tick') for (const name of ['duration_ms', 'schedule_delay_ms']) add(group, `tick_${name}`, row[name]);
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
    if (row.time < start || row.time > end) continue;
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
  return { valid: errors.length === 0, errors, trace: traceEnd, measured_seconds: (end - start) / 1000, intent_observations: outcomes, resources, database, committed_games: committedGames, retention,
    metrics: Object.fromEntries([...groups].map(([group, metrics]) => [group, Object.fromEntries([...metrics].map(([name, metric]) => [name, metric.summary()]))])) };
}

// Called only for new harness runs. Historical trace analysis stays compatible.
export function validateRun(result, metadata) {
  const reject = message => { result.errors.push(message); result.valid = false; };
  if (!metadata.measurement_completed) reject('Measurement window did not complete');
  for (const game of metadata.games ?? []) {
    if (!(result.committed_games[game.id] > 0)) reject(`Missing committed progress: ${game.id}`);
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
