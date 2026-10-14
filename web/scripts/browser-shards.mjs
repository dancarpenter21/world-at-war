import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const count = Number(process.env.E2E_SHARDS ?? "3");
if (!Number.isInteger(count) || count < 1 || count > 8) throw new Error("E2E_SHARDS must be an integer from 1 to 8");
const output = path.resolve("test-results", `parallel-${Date.now()}`);
await mkdir(output, { recursive: true });
const children = new Set();
let interrupted = false;
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => {
  interrupted = true;
  for (const child of children) child.kill(signal);
});
function run(index, args) {
  return new Promise(resolve => {
    const child = spawn(process.execPath, ["node_modules/@playwright/test/cli.js", "test", ...args, ...process.argv.slice(2)], {
      stdio: "inherit",
      env: { ...process.env, E2E_FRONTEND_PORT: String(4173 + index), E2E_BACKEND_PORT: String(18101 + index),
        E2E_OUTPUT_DIR: path.join(output, String(index + 1)), PLAYWRIGHT_HTML_OUTPUT_DIR: path.join(output, `html-${index + 1}`) }
    });
    children.add(child);
    child.once("error", error => { console.error(error); children.delete(child); resolve(1); });
    child.once("exit", code => { children.delete(child); resolve(code ?? 1); });
  });
}
console.log(`Running ${count} isolated browser shards. Artifacts: ${output}`);
const results = await Promise.all(Array.from({ length: count }, (_, index) =>
  run(index, [`--shard=${index + 1}/${count}`, "--grep-invert=durable accounts and recovery"])));
// Persistence tests share a supplied database; keep those together after the shards.
if (!interrupted && process.env.PERSISTENCE_E2E_DATABASE_URL) results.push(await run(count, ["persistence.e2e.ts"]));
process.exitCode = interrupted ? 130 : results.some(code => code !== 0) ? 1 : 0;
