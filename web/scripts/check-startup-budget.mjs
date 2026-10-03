import { readFile, stat } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const dist = fileURLToPath(new URL("../dist/", import.meta.url));
const manifest = JSON.parse(await readFile(resolve(dist, ".vite/manifest.json"), "utf8"));
const entry = Object.keys(manifest).find((key) => manifest[key].isEntry && manifest[key].src === "index.html");
if (!entry) throw new Error("The web build has no index.html entry in its Vite manifest.");

// Count every static JavaScript dependency, even when Rollup moves it to another chunk.
// Dynamic map and graph imports do not block the lobby's startup.
const visited = new Set();
async function startupBytes(key) {
  if (visited.has(key)) return 0;
  visited.add(key);
  const chunk = manifest[key];
  if (!chunk) throw new Error(`Missing startup dependency in the Vite manifest: ${key}`);
  const file = resolve(dist, chunk.file);
  if (dirname(file) !== resolve(dist, "assets")) throw new Error(`Unexpected startup chunk path: ${chunk.file}`);
  let bytes = (await stat(file)).size;
  for (const dependency of chunk.imports ?? []) bytes += await startupBytes(dependency);
  return bytes;
}

const bytes = await startupBytes(entry);
const budget = 350 * 1_024;
console.log(`Startup JavaScript: ${(bytes / 1_024).toFixed(1)} KiB / ${budget / 1_024} KiB budget (${visited.size} static chunks).`);
if (bytes > budget) throw new Error("Startup JavaScript exceeds the lobby budget. Keep map and graph dependencies behind dynamic imports.");
