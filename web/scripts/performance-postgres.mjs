import { spawn } from 'node:child_process';
import { randomBytes, randomUUID } from 'node:crypto';

// Never include arguments, environment, or subprocess stderr in errors: they may
// contain a connection secret. Commands use argv, never a shell.
export function docker(args, { env = process.env, input = '', timeout = 30_000 } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn('docker', args, { env, stdio: ['pipe', 'pipe', 'pipe'] });
    let output = '', overflow = false;
    const timer = setTimeout(() => child.kill('SIGKILL'), timeout);
    child.stdout.on('data', chunk => { output += chunk; if (output.length > 1024 * 1024) { overflow = true; child.kill('SIGKILL'); } });
    child.stderr.resume();
    child.stdin.on('error', () => {});
    child.on('error', () => { clearTimeout(timer); reject(new Error('Docker could not be started')); });
    child.on('close', code => { clearTimeout(timer); if (code === 0 && !overflow) resolve(output.trim()); else reject(new Error(`Docker ${args[0]} failed`)); });
    child.stdin.end(input);
  });
}

export class PerformancePostgres {
  constructor(image = 'postgres:17-alpine', run = docker) {
    this.run = run; this.image = image;
    this.name = `waw-perf-${randomUUID()}`; this.volume = `${this.name}-data`;
    this.password = randomBytes(24).toString('hex');
    this.volumeCreated = false; this.containerAttempted = false;
  }
  async sql(query) {
    return this.run(['exec', '-i', '--env', 'PGPASSWORD', this.name, 'psql', '-X', '-A', '-t', '-v', 'ON_ERROR_STOP=1', '-h', '127.0.0.1', '-U', 'waw_perf', '-d', 'waw_perf'], { input: query, env: { ...process.env, PGPASSWORD: this.password } });
  }
  async start() {
    await this.run(['info', '--format', '{{.ServerVersion}}']);
    try { await this.run(['image', 'inspect', this.image]); }
    catch { await this.run(['pull', this.image], { timeout: 180_000 }); }
    const [image] = JSON.parse(await this.run(['image', 'inspect', this.image]));
    this.imageId = image.Id;
    // Creation can succeed in Docker even if the CLI is interrupted before
    // returning. Remember the owned name before issuing the command.
    this.volumeCreated = true;
    await this.run(['volume', 'create', '--label', 'world-at-war.performance=true', this.volume]);
    this.containerAttempted = true;
    await this.run(['run', '--detach', '--name', this.name, '--label', 'world-at-war.performance=true',
      '--publish', '127.0.0.1::5432', '--mount', `type=volume,source=${this.volume},target=/var/lib/postgresql/data`,
      '--env', 'POSTGRES_USER=waw_perf', '--env', 'POSTGRES_DB=waw_perf', '--env', 'POSTGRES_PASSWORD', this.imageId],
    { env: { ...process.env, POSTGRES_PASSWORD: this.password } });
    const deadline = performance.now() + 60_000;
    while (true) {
      try { await this.sql('SELECT 1;'); break; }
      catch { if (performance.now() >= deadline) throw new Error('Performance PostgreSQL readiness timed out'); await new Promise(r => setTimeout(r, 250)); }
    }
    const port = await this.run(['port', this.name, '5432/tcp']);
    if (!/^127\.0\.0\.1:\d+$/.test(port)) throw new Error('Performance PostgreSQL is not loopback-only');
    this.url = `postgres://waw_perf:${this.password}@${port}/waw_perf`;
    this.initialLsn = await this.sql('SELECT pg_current_wal_lsn();');
    if (!/^[0-9A-F]+\/[0-9A-F]+$/.test(this.initialLsn)) throw new Error('Invalid PostgreSQL WAL position');
    const settings = JSON.parse(await this.sql(`SELECT json_build_object('version',version(), 'fsync',current_setting('fsync'),
      'synchronous_commit',current_setting('synchronous_commit'), 'full_page_writes',current_setting('full_page_writes'),
      'shared_buffers',current_setting('shared_buffers'), 'checkpoint_timeout',current_setting('checkpoint_timeout'), 'max_wal_size',current_setting('max_wal_size'),
      'wal_buffers',current_setting('wal_buffers'), 'track_io_timing',current_setting('track_io_timing'),
      'track_wal_io_timing',current_setting('track_wal_io_timing'));`));
    if (['fsync', 'synchronous_commit', 'full_page_writes'].some(k => settings[k] !== 'on')) throw new Error('PostgreSQL durability settings must be enabled');
    return { container_name: this.name, volume_name: this.volume, image_id: image.Id, image_digests: image.RepoDigests, settings, storage: 'fresh Docker named volume', sample_seconds: 5 };
  }
  async sample() {
    const started = performance.now();
    const values = JSON.parse(await this.sql(`SELECT json_build_object('database_bytes', pg_database_size(current_database()),
      'checkpointer',(SELECT row_to_json(c) FROM pg_stat_checkpointer c),
      'wal_stats',(SELECT row_to_json(w) FROM pg_stat_wal w),
      'io_stats',(SELECT json_agg(i) FROM pg_stat_io i),
      'activity_waits',(SELECT COALESCE(json_agg(w),'[]'::json) FROM (SELECT wait_event_type,wait_event,COUNT(*) AS sessions FROM pg_stat_activity WHERE backend_type='client backend' AND pid<>pg_backend_pid() GROUP BY wait_event_type,wait_event) w),
      'wal_bytes',pg_wal_lsn_diff(pg_current_wal_lsn(),'${this.initialLsn}'),
      'relations',COALESCE((SELECT json_object_agg(relname,json_build_object('total_bytes',pg_total_relation_size(relid),
        'table_bytes',pg_table_size(relid),'index_bytes',pg_indexes_size(relid))) FROM pg_stat_user_tables),'{}'::json));`));
    const stats = JSON.parse(await this.run(['stats', '--no-stream', '--format', '{{json .}}', this.name]));
    return { kind: 'database_resource', ...values, container_cpu_percent: stats.CPUPerc,
      container_memory: stats.MemUsage, container_block_io: stats.BlockIO, sampling_ms: performance.now() - started };
  }
  async stop() {
    // Do not remove a volume if its container could not be stopped.
    if (this.containerAttempted) {
      const found = await this.run(['ps', '-aq', '--filter', `name=^/${this.name}$`]);
      if (found) await this.run(['rm', '--force', this.name]);
      this.containerAttempted = false;
    }
    if (this.volumeCreated) {
      const found = await this.run(['volume', 'ls', '-q', '--filter', `name=^${this.volume}$`]);
      if (found) await this.run(['volume', 'rm', this.volume]);
      this.volumeCreated = false;
    }
  }
}
