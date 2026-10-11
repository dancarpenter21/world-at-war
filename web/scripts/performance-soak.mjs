import { spawn } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { finished } from 'node:stream/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const help = `Unattended soak tests (Node 22+, Linux/WSL).
Usage: node web/scripts/performance-soak.mjs [options]
  --storage both|volatile|postgres  Default: both, sequential fresh runs
  --profile mixed|regional|global  Default: mixed (50 clients/five games)
  --duration SECONDS               Default: 3600 per storage mode (max 86400)
  --warmup SECONDS                 Default: 60
  --reconnect-every SECONDS        Default: 300
  --clients N                     Optional profile player count (1..200)
  --seed N                        Default: 12787750
  --postgres-image IMAGE          Default: postgres:17-alpine
  --output DIRECTORY              Fresh directory; default timestamp under tmp/performance
  --timeout-grace SECONDS          Default: 600 beyond warmup + duration
  --check-budgets                  Fail on applicable timing budget misses
  --help                          Print this help
Build the release server and install web dependencies before running.
Logs, status.json, report.json and report.md are written to the output directory.
Nonzero exit means an invalid, interrupted, timed-out or failed run. No automatic retry.
`;

export function options(args) {
  const { values } = parseArgs({ args, options: {
    storage: { type: 'string', default: 'both' }, profile: { type: 'string', default: 'mixed' },
    duration: { type: 'string', default: '3600' }, warmup: { type: 'string', default: '60' },
    'reconnect-every': { type: 'string', default: '300' }, seed: { type: 'string', default: '12787750' },
    clients: { type: 'string' }, 'postgres-image': { type: 'string', default: 'postgres:17-alpine' },
    output: { type: 'string' }, 'timeout-grace': { type: 'string', default: '600' },
    'check-budgets': { type: 'boolean', default: false }, help: { type: 'boolean', default: false },
  } });
  if (values.help) return values;
  if (!['both', 'volatile', 'postgres'].includes(values.storage)) throw new Error('Invalid storage');
  if (!['mixed', 'regional', 'global'].includes(values.profile)) throw new Error('Invalid profile');
  for (const [key, min, max] of [['duration', 1, 86400], ['warmup', 0, 3600], ['reconnect-every', 15, 86400], ['seed', 0, Number.MAX_SAFE_INTEGER], ['timeout-grace', 1, 3600], ['clients', 1, 200]]) {
    if (values[key] === undefined) continue;
    const n = Number(values[key]);
    if (!Number.isSafeInteger(n) || n < min || n > max) throw new Error(`${key} must be ${min}..${max}`);
  }
  return values;
}

export async function runSoak(values, harness = path.join(root, 'web/scripts/performance-server.mjs')) {
  const output = path.resolve(values.output ?? path.join(root, 'tmp/performance', `soak-${new Date().toISOString().replaceAll(':', '-')}`));
  await mkdir(path.dirname(output), { recursive: true });
  await mkdir(output); // Never overwrite an earlier run.
  const report = { started_at: new Date().toISOString(), output, configuration: values, state: 'running', runs: [] };
  const journal = createWriteStream(path.join(output, 'runner.log'), { flags: 'wx' });
  let child, stopChild, interrupted = false;
  const log = message => { const line = `${new Date().toISOString()} ${message}\n`; journal.write(line); process.stdout.write(line); };
  const save = async () => {
    await writeFile(path.join(output, 'status.json.tmp'), JSON.stringify(report, null, 2) + '\n');
    await rename(path.join(output, 'status.json.tmp'), path.join(output, 'status.json'));
  };
  const interrupt = () => { interrupted = true; log('Interrupted; waiting for harness cleanup'); stopChild?.(); };
  journal.on('error', error => { report.error = `Runner log: ${error.message}`; interrupted = true; stopChild?.(); });
  process.on('SIGINT', interrupt); process.on('SIGTERM', interrupt);
  try {
    await save();
    for (const storage of values.storage === 'both' ? ['volatile', 'postgres'] : [values.storage]) {
      if (interrupted) break;
      const directory = path.join(output, `${values.profile}-${storage}`);
      const run = { storage, directory, started_at: new Date().toISOString(), state: 'running' };
      report.runs.push(run); await save(); log(`START ${storage}; artifacts: ${directory}`);
      const args = [harness, '--storage', storage, '--profile', values.profile, '--output', directory];
      for (const key of ['duration', 'warmup', 'reconnect-every', 'seed', 'clients', 'postgres-image']) if (values[key] !== undefined) args.push(`--${key}`, values[key]);
      if (values['check-budgets']) args.push('--check-budgets');
      const stream = createWriteStream(path.join(output, `${storage}.log`), { flags: 'wx' });
      let timedOut = false, spawnError, forceTimer;
      child = spawn(process.execPath, args, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
      child.stdout.pipe(stream, { end: false }); child.stderr.pipe(stream, { end: false });
      stopChild = () => {
        child?.kill('SIGTERM');
        if (forceTimer) return;
        forceTimer = setTimeout(() => { log(`Cleanup stalled for ${storage}; forcing exit. Check metadata for owned Docker resources.`); child.kill('SIGKILL'); }, 120_000);
      };
      stream.on('error', error => { spawnError = `Harness log: ${error.message}`; stopChild(); });
      const timeout = setTimeout(() => { timedOut = true; log(`TIMEOUT ${storage}; requesting cleanup`); stopChild(); }, (Number(values.duration) + Number(values.warmup) + Number(values['timeout-grace'])) * 1000);
      const heartbeat = setInterval(() => log(`RUNNING ${storage}; elapsed ${Math.round((Date.now() - Date.parse(run.started_at)) / 1000)}s; progress in ${storage}.log`), 60_000);
      const result = await new Promise(resolve => {
        child.on('error', error => { spawnError = error.message; });
        child.on('close', (code, signal) => resolve({ code, signal }));
      });
      clearTimeout(timeout); clearTimeout(forceTimer); clearInterval(heartbeat); child = undefined; stopChild = undefined;
      stream.end(); await finished(stream);
      let summary, metadata, artifactError;
      try { summary = JSON.parse(await readFile(path.join(directory, 'summary.json'))); metadata = JSON.parse(await readFile(path.join(directory, 'metadata.json'))); }
      catch (error) { artifactError = `Missing or unreadable results: ${error.message}`; }
      const misses = Object.values(summary?.budgets ?? {}).flat().filter(b => !b.met);
      Object.assign(run, { ended_at: new Date().toISOString(), exit_code: result.code, signal: result.signal, timed_out: timedOut,
        state: !interrupted && !timedOut && result.code === 0 && summary?.valid === true && metadata?.measurement_completed === true && (!values['check-budgets'] || misses.length === 0) ? 'passed' : 'failed',
        errors: [spawnError, artifactError, timedOut ? 'Watchdog timeout' : undefined, interrupted ? 'Run interrupted' : undefined, ...(summary?.errors ?? [])].filter(Boolean), budget_misses: misses,
        cycle: summary?.metrics?.server?.cycle_duration_ms, trace: summary?.trace,
        reconnect_clients: summary?.budgets?.reconnect?.length ?? 0, resources: summary?.resources, database: summary?.database, retention: summary?.retention,
      });
      await save(); log(`END ${storage}: ${run.state}; cycle p95=${run.cycle?.p95 ?? 'missing'} ms; budget misses=${misses.length}`);
      // An invalid measurement does not prevent the other independent mode running.
    }
  } catch (error) { report.error = error.message; log(`ERROR ${error.message}`); }
  finally {
    process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
    report.ended_at = new Date().toISOString();
    report.state = interrupted ? 'interrupted' : !report.error && report.runs.length > 0 && report.runs.every(r => r.state === 'passed') ? 'passed' : 'failed';
    await save();
    await writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2) + '\n');
    const lines = [`# Soak test: ${report.state}`, '', `Started: ${report.started_at}`, `Finished: ${report.ended_at}`, '', '| Storage | Result | Cycle p95 ms | Cycle max ms | Budget misses |', '| --- | --- | ---: | ---: | ---: |',
      ...report.runs.map(r => `| ${r.storage} | ${r.state} | ${r.cycle?.p95 ?? 'missing'} | ${r.cycle?.max ?? 'missing'} | ${r.budget_misses?.length ?? 'unknown'} |`), '',
      'See each run’s summary.json for resource growth, retention, reconnect coverage and errors.',
      ...report.runs.flatMap(r => ['', `## ${r.storage}`, '', ...(r.errors ?? []).map(e => `- ${e}`), `- Reconnect clients sampled: ${r.reconnect_clients ?? 0}`, ...['server_rss_bytes', 'audit_bytes'].map(key => { const v = (r.resources ?? []).map(row => row[key]).filter(Number.isFinite); return `- ${key}, first / last / peak: ${v.length ? [v[0], v.at(-1), Math.max(...v)].join(' / ') : 'missing'} bytes`; }), ...['database_bytes', 'wal_bytes'].map(key => { const v = (r.database ?? []).map(row => row[key]).filter(Number.isFinite); return `- ${key}, first / last / peak: ${v.length ? [v[0], v.at(-1), Math.max(...v)].join(' / ') : 'not available'} bytes`; })]), '',
      'Budget misses are findings unless --check-budgets was supplied. Passing does not establish bounded memory or 24-hour stability.',
      'Interrupted/forcibly killed processes may leave resources: inspect metadata.json and remove only its named container/volume.', '', ...(report.error ? [report.error, ''] : [])];
    await writeFile(path.join(output, 'report.md'), lines.join('\n') + '\n');
    log(`FINISHED ${report.state}; report: ${path.join(output, 'report.md')}`);
    journal.end(); await finished(journal);
  }
  return report.state === 'passed' ? 0 : interrupted ? 130 : 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { const values = options(process.argv.slice(2)); if (values.help) console.log(help); else process.exitCode = await runSoak(values); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
