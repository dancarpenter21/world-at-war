import { installFixtureTerrain } from "./terrain";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { writeFile } from "node:fs/promises";

export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
export const backendUrl = `http://${process.env.E2E_BROWSER_HOST ?? "127.0.0.1"}:${process.env.E2E_BACKEND_PORT ?? "18101"}`;
const healthUrl = `http://127.0.0.1:${process.env.E2E_BACKEND_PORT ?? "18101"}/health`;

export async function startBackend(environment: Record<string, string> = {}) {
  // Fail rather than accidentally test an unrelated process occupying our fixed frontend API port.
  const occupied = await fetch(healthUrl).then(() => true, () => false);
  if (occupied) throw new Error(`E2E backend port ${process.env.E2E_BACKEND_PORT ?? "18101"} is already occupied`);
  const directory = await mkdtemp(path.join(tmpdir(), "world-at-war-e2e-"));
  const terrain = await installFixtureTerrain(directory);
  const airports = [JSON.parse(await readFile(new URL("../fixtures/airport.json", import.meta.url), "utf8"))];
  const airportCache = path.join(directory, "airports");
  await mkdir(airportCache);
  await writeFile(path.join(airportCache, "latest.json"), JSON.stringify({ schema_version: 1,
    synced_unix: Math.floor(Date.now() / 1000), checksum: createHash("sha256").update(JSON.stringify(airports)).digest("hex"),
    sources: [], degraded_sources: [], airports }));
  let output = "";
  let startError: Error | undefined;
  function launch() {
    startError = undefined;
    const processHandle = spawn(path.join(repoRoot, "target/debug/world-at-war-server"), [], {
      cwd: directory,
      env: { ...process.env, BIND_ADDR: `0.0.0.0:${process.env.E2E_BACKEND_PORT ?? "18101"}`, APP_ALLOWED_ORIGINS: `http://${process.env.E2E_BROWSER_HOST ?? "127.0.0.1"}:${process.env.E2E_FRONTEND_PORT ?? "4173"}`, ADMIN_SETUP_TOKEN: "", DATABASE_URL: "", COOKIE_SECURE: "false",
        SPACETRACK_USERNAME: "", SPACETRACK_PASSWORD: "", SPACETRACK_LOGIN_URL: "http://127.0.0.1:1/login", SPACETRACK_GP_URL: "http://127.0.0.1:1/gp",
        SPACE_CARDS_DIR: path.join(directory, "cards"), AIRPORT_CACHE_DIR: airportCache,
        AIRPORT_REFRESH_MAX_AGE_SECONDS: "86400", OURAIRPORTS_AIRPORTS_URL: "http://127.0.0.1:1/airports",
        OURAIRPORTS_RUNWAYS_URL: "http://127.0.0.1:1/runways", FAA_NASR_SUBSCRIPTION_URL: "http://127.0.0.1:1/faa", FAA_NASR_APT_URL: "",
        TERRAIN_DIR: terrain,
        COMMUNICATIONS_CATALOG_PATH: path.join(repoRoot, "data/communications/catalog.yaml"), NETWORK_EVENT_DIR: path.join(directory, "events"),
        ...environment }, stdio: ["ignore", "pipe", "pipe"]
    });
    processHandle.on("error", error => { startError = error; });
    processHandle.stdout.on("data", chunk => { output = (output + String(chunk)).slice(-100_000); });
    processHandle.stderr.on("data", chunk => { output = (output + String(chunk)).slice(-100_000); });
    return processHandle;
  }
  let processHandle = launch();
  let stopped = false;
  async function stopProcess(signal: "SIGTERM" | "SIGKILL" = "SIGTERM") {
    if (processHandle.pid && processHandle.exitCode === null && processHandle.signalCode === null) {
      const exited = once(processHandle, "exit");
      processHandle.kill(signal);
      const timer = setTimeout(() => processHandle.kill("SIGKILL"), 2000);
      try { await exited; } finally { clearTimeout(timer); }
    }
  }
  async function stop() {
    if (stopped) return;
    stopped = true;
    await stopProcess();
    await rm(directory, { recursive: true, force: true });
  }
  async function restart(signal: "SIGTERM" | "SIGKILL" = "SIGTERM") {
    await stopProcess(signal);
    processHandle = launch();
    await ready();
  }
  async function ready() {
    try {
      const deadline = Date.now() + 30_000;
      while (Date.now() < deadline) {
        if (startError) throw startError;
        if (processHandle.exitCode !== null || processHandle.signalCode !== null) throw new Error(`Backend exited:\n${output}`);
        if (await fetch(healthUrl).then(response => response.ok, () => false)) return;
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      throw new Error(`Backend startup timed out:\n${output}`);
    } catch (error) { await stop(); throw error; }
  }
  await ready();
  return { directory, stop, restart, output: () => output };
}
export type Backend = Awaited<ReturnType<typeof startBackend>>;
