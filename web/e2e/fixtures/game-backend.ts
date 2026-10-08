import { spawn, type ChildProcess } from "node:child_process";
import { createServer, type Server } from "node:http";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");

export async function startGameBackend() {
  const runDirectory = await mkdtemp(path.join(tmpdir(), "world-at-war-game-e2e-"));
  let backend: ChildProcess | undefined;
  let provider: Server | undefined;
  let output = "";
  const url = "http://127.0.0.1:18101";
  const close = async () => {
    if (backend && backend.exitCode === null && !backend.killed) {
      const stopped = new Promise<void>((resolve) => backend!.once("exit", () => resolve()));
      backend.kill("SIGTERM");
      await Promise.race([stopped, new Promise<void>((resolve) => setTimeout(resolve, 2_000))]);
    }
    if (provider?.listening) await new Promise<void>((resolve) => provider!.close(() => resolve()));
    const resolved = path.resolve(runDirectory);
    if (path.dirname(resolved) !== path.resolve(tmpdir()) || !path.basename(resolved).startsWith("world-at-war-game-e2e-")) {
      throw new Error("Refusing to remove an unexpected test directory.");
    }
    await rm(resolved, { recursive: true, force: true });
  };

  try {
    provider = createServer((request, response) => {
      if (request.method === "POST" && request.url === "/ajaxauth/login") {
        request.resume();
        response.writeHead(200, { "content-type": "application/json", "set-cookie": "fixture-session=1; Path=/; HttpOnly" });
        response.end(JSON.stringify({ Login: "Success" }));
      } else if (request.method === "GET" && request.url?.startsWith("/basicspacedata/")) {
        response.writeHead(200, { "content-type": "application/json" });
        response.end(JSON.stringify([
          { NORAD_CAT_ID: "25544", OBJECT_NAME: "ISS (ZARYA)", OBJECT_TYPE: "PAYLOAD" },
          { NORAD_CAT_ID: "5", OBJECT_NAME: "VANGUARD 1", OBJECT_TYPE: "PAYLOAD" }
        ]));
      } else if (request.url === "/airports.csv") {
        response.writeHead(200, { "content-type": "text/csv" });
        response.end("id,ident,type,name,latitude_deg,longitude_deg,elevation_ft,iso_country,iso_region,municipality,gps_code,icao_code,iata_code,local_code\n1,KAAA,large_airport,Fixture Airport,10,-20,100,US,US-CA,Town,KAAA,KAAA,AAA,AAA\n");
      } else if (request.url === "/runways.csv") {
        response.writeHead(200, { "content-type": "text/csv" });
        response.end("id,airport_ref,length_ft,width_ft,surface,lighted,closed,le_ident,le_latitude_deg,le_longitude_deg,le_elevation_ft,le_displaced_threshold_ft,he_ident,he_latitude_deg,he_longitude_deg,he_elevation_ft,he_displaced_threshold_ft\n2,1,10000,150,ASPH,1,0,09,10,-20,100,500,27,10.1,-20.1,101,\n");
      } else {
        response.writeHead(404); response.end();
      }
    });
    await new Promise<void>((resolve, reject) => {
      provider!.once("error", reject);
      provider!.listen(0, "127.0.0.1", resolve);
    });
    const address = provider.address();
    if (!address || typeof address === "string") throw new Error("Mock provider has no TCP address.");
    backend = spawn(path.join(repoRoot, "target/debug", process.platform === "win32" ? "world-at-war-server.exe" : "world-at-war-server"), [], {
      cwd: runDirectory, windowsHide: true,
      env: {
        ...process.env,
        BIND_ADDR: "0.0.0.0:18101", APP_ALLOWED_ORIGINS: `http://${process.env.E2E_BROWSER_HOST ?? "127.0.0.1"}:4173`, ADMIN_SETUP_TOKEN: "",
        SPACETRACK_USERNAME: "", SPACETRACK_PASSWORD: "",
        SPACETRACK_LOGIN_URL: `http://127.0.0.1:${address.port}/ajaxauth/login`,
        SPACETRACK_GP_URL: `http://127.0.0.1:${address.port}/basicspacedata/query/class/gp/format/json`,
        COMMUNICATIONS_CATALOG_PATH: path.join(repoRoot, "data/communications/catalog.yaml"),
        NETWORK_EVENT_DIR: path.join(runDirectory, "var/network-events"),
        AIRPORT_CACHE_DIR: path.join(runDirectory, "data/cache/airports"),
        OURAIRPORTS_AIRPORTS_URL: `http://127.0.0.1:${address.port}/airports.csv`,
        OURAIRPORTS_RUNWAYS_URL: `http://127.0.0.1:${address.port}/runways.csv`,
        FAA_NASR_SUBSCRIPTION_URL: `http://127.0.0.1:${address.port}/faa`,
        FAA_NASR_APT_URL: `http://127.0.0.1:${address.port}/faa.zip`,
        COOKIE_SECURE: "false"
      },
      stdio: ["ignore", "pipe", "pipe"]
    });
    backend.stdout?.on("data", (chunk) => { output += String(chunk); });
    backend.stderr?.on("data", (chunk) => { output += String(chunk); });
    let spawnError: Error | undefined;
    backend.on("error", (error) => { spawnError = error; });
    const deadline = Date.now() + 30_000;
    while (true) {
      if (spawnError) throw spawnError;
      if (backend.exitCode !== null) throw new Error(`Backend exited before ready:\n${output}`);
      const healthy = await fetch(url + "/health").then((response) => response.ok).catch(() => false);
      if (healthy) break;
      if (Date.now() >= deadline) throw new Error(`Backend failed to start:\n${output}`);
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    return { url, runDirectory, close };
  } catch (error) {
    await close();
    throw error;
  }
}
