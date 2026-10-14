import { defineConfig } from "@playwright/test";

const wsEndpoint = process.env.PLAYWRIGHT_WS_ENDPOINT;
const frontendPort = process.env.E2E_FRONTEND_PORT ?? "4173";
const backendPort = process.env.E2E_BACKEND_PORT ?? "18101";
const browserHost = process.env.E2E_BROWSER_HOST ?? "127.0.0.1";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.e2e.ts",
  // Shard individual tests evenly; each runner still executes one at a time.
  fullyParallel: true,
  workers: 1,
  timeout: 60_000,
  outputDir: process.env.E2E_OUTPUT_DIR ?? "test-results",
  use: {
    baseURL: `http://${browserHost}:${frontendPort}`,
    browserName: "chromium",
    actionTimeout: 15_000,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    ...(wsEndpoint ? { connectOptions: { wsEndpoint } } : {})
  },
  webServer: {
    command: `npm run dev -- --host 0.0.0.0 --port ${frontendPort} --strictPort`,
    url: `http://127.0.0.1:${frontendPort}`,
    reuseExistingServer: false,
    env: {
      VITE_API_BASE: `http://${browserHost}:${backendPort}`,
      // Isolated headless player contexts all remain visible/focused. Bound
      // software rendering for functional tests while keeping real canvases,
      // interpolation, polling, streams and the server clock active.
      VITE_MAP_FRAME_RATE: process.env.E2E_MAP_FRAME_RATE ?? "5",
      VITE_MAP_RESOLUTION_SCALE: process.env.E2E_MAP_RESOLUTION_SCALE ?? "0.25"
    }
  }
});
