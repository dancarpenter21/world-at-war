import { defineConfig } from "@playwright/test";

const wsEndpoint = process.env.PLAYWRIGHT_WS_ENDPOINT;
const browserHost = process.env.E2E_BROWSER_HOST ?? "127.0.0.1";

export default defineConfig({
  testDir: "./e2e",
  testMatch: "**/*.e2e.ts",
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  use: {
    baseURL: `http://${browserHost}:4173`,
    browserName: "chromium",
    ...(wsEndpoint ? { connectOptions: { wsEndpoint } } : {})
  },
  webServer: {
    command: "npm run dev -- --host 0.0.0.0 --port 4173 --strictPort",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
    env: {
      VITE_API_BASE: `http://${browserHost}:18101`
    }
  }
});
