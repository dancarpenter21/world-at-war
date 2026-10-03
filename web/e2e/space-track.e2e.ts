import { expect, test } from "@playwright/test";
import { createServer, type Server } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { startBackend, backendUrl, type Backend } from "./support/server";

const liveUsername = process.env.SPACETRACK_E2E_USERNAME;
const livePassword = process.env.SPACETRACK_E2E_PASSWORD;
const useLiveProvider = Boolean(liveUsername && livePassword);
const savedPasswordMask = "••••••••••••";

let backend: Backend;
let mockProvider: Server | undefined;
let runDirectory: string;
let mockLoginReceived = false;
let mockCatalogDownloaded = false;

async function startMockProvider(): Promise<number> {
  mockProvider = createServer((request, response) => {
    if (request.method === "POST" && request.url === "/ajaxauth/login") {
      let body = "";
      request.setEncoding("utf8");
      request.on("data", (chunk) => { body += chunk; });
      request.on("end", () => {
        const form = new URLSearchParams(body);
        mockLoginReceived = form.get("identity") === "integration-user"
          && form.get("password") === "integration-password";
        response.writeHead(mockLoginReceived ? 200 : 403, {
          "content-type": "application/json",
          "set-cookie": "space-track-session=integration-session; Path=/; HttpOnly"
        });
        response.end(JSON.stringify({ Login: mockLoginReceived ? "Success" : "Failed" }));
      });
      return;
    }

    if (request.method === "GET" && request.url?.startsWith("/basicspacedata/query/class/gp/")) {
      mockCatalogDownloaded = request.headers.cookie?.includes(
        "space-track-session=integration-session"
      ) ?? false;
      if (!mockCatalogDownloaded) {
        response.writeHead(302, { location: "/auth/login" });
        response.end();
        return;
      }
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify([
        { NORAD_CAT_ID: "25544", OBJECT_NAME: "ISS (ZARYA)", OBJECT_TYPE: "PAYLOAD" },
        { NORAD_CAT_ID: "5", OBJECT_NAME: "VANGUARD 1", OBJECT_TYPE: "PAYLOAD" }
      ]));
      return;
    }

    response.writeHead(404);
    response.end();
  });

  await new Promise<void>((resolve, reject) => {
    mockProvider!.once("error", reject);
    mockProvider!.listen(0, "127.0.0.1", resolve);
  });
  const address = mockProvider.address();
  if (!address || typeof address === "string") throw new Error("mock provider has no TCP port");
  return address.port;
}

test.beforeAll(async () => {
  if (Boolean(liveUsername) !== Boolean(livePassword)) {
    throw new Error("set both SPACETRACK_E2E_USERNAME and SPACETRACK_E2E_PASSWORD");
  }

  const providerEnvironment: Record<string, string> = useLiveProvider ? { SPACETRACK_LOGIN_URL: "https://www.space-track.org/ajaxauth/login", SPACETRACK_GP_URL: "https://www.space-track.org/basicspacedata/query/class/gp/decay_date/null-val/epoch/%3Enow-10/orderby/norad_cat_id/format/json" } : {};
  if (!useLiveProvider) {
    const providerPort = await startMockProvider();
    providerEnvironment.SPACETRACK_LOGIN_URL = `http://127.0.0.1:${providerPort}/ajaxauth/login`;
    providerEnvironment.SPACETRACK_GP_URL = `http://127.0.0.1:${providerPort}/basicspacedata/query/class/gp/decay_date/null-val/epoch/%3Enow-10/orderby/norad_cat_id/format/json`;
  }

  backend = await startBackend(providerEnvironment);
  runDirectory = backend.directory;
});

test.afterAll(async () => {
  await backend?.stop();
  if (mockProvider) await new Promise<void>((resolve) => mockProvider!.close(() => resolve()));
});

test("downloads and persists a Space-Track catalog through the login form", async ({ page }) => {
  await page.addInitScript(() => {
    if (!globalThis.crypto.randomUUID) {
      Object.defineProperty(globalThis.crypto, "randomUUID", {
        value: () => "00000000-0000-4000-8000-000000000001"
      });
    }
  });
  page.on("console", (message) => console.log(`browser console: ${message.type()}: ${message.text()}`));
  page.on("pageerror", (error) => console.log(`browser page error: ${error.message}`));
  page.on("requestfailed", (request) => {
    console.log(`browser request failed: ${request.url()} (${request.failure()?.errorText})`);
  });
  const navigation = await page.goto("/");
  expect(navigation?.status()).toBe(200);
  const spaceConfigurationTab = page.getByRole("button", { name: "Space Configuration", exact: true });
  await expect(spaceConfigurationTab).toBeVisible({ timeout: 15_000 });
  await expect(page.locator(".catalog-tab-status.missing")).toBeVisible();
  await spaceConfigurationTab.click();
  await expect(page.getByLabel("Space-Track username")).toBeVisible({ timeout: 15_000 });
  await page.getByLabel("Space-Track username").fill(liveUsername ?? "integration-user");
  await page.getByLabel("Space-Track password").fill(livePassword ?? "integration-password");
  await expect(page.getByRole("button", { name: "Connect and synchronize", exact: true })).toBeEnabled();

  const connectResponse = page.waitForResponse((response) =>
    response.url() === `${backendUrl}/v1/admin/space-track/connect`
  );
  await page.getByRole("button", { name: "Connect and synchronize" }).click();
  expect((await connectResponse).status()).toBe(200);
  await expect(page.getByText(/Catalog ready: [\d,]+ public objects\./)).toBeVisible();
  const successFeedback = page.getByRole("status").filter({ hasText: "Catalog download complete" });
  await expect(successFeedback).toBeVisible();
  await expect(successFeedback).toContainText(/\d[\d,]* public objects are ready to use\./);
  await expect(page.getByText("Last downloaded", { exact: true })).toBeVisible();
  await expect(page.locator(".space-catalog-timestamp time")).toHaveAttribute("datetime", /^\d{4}-\d{2}-\d{2}T/);
  await expect(page.locator(".space-catalog-timestamp")).toContainText("just now");
  await expect(page.locator(".catalog-tab-status.ready")).toBeVisible();
  const cooldownButton = page.getByRole("button", { name: /Refresh available in \d+m \d{2}s/ });
  await expect(cooldownButton).toBeDisabled();
  await expect(page.getByText("Catalog downloads are limited to once per hour after a successful refresh.")).toBeVisible();

  await page.reload();
  await page.getByRole("button", { name: "Space Configuration", exact: true }).click();
  await expect(page.getByLabel("Space-Track username")).toHaveValue(liveUsername ?? "integration-user");
  await expect(page.getByLabel("Space-Track password")).toHaveValue(savedPasswordMask);
  await expect(page.getByText("Credentials saved for 30 days")).toBeVisible();
  await page.getByLabel("Space-Track password").focus();
  await expect(page.getByLabel("Space-Track password")).toHaveValue("");
  await page.getByRole("heading", { name: "Space-Track setup" }).click();
  await expect(page.getByLabel("Space-Track password")).toHaveValue(savedPasswordMask);

  await page.getByRole("button", { name: "New scenario" }).click();
  await page.getByRole("button", { name: "Create game" }).click();
  await expect(page.getByText("Claim a command role.")).toBeVisible();
  await page.getByRole("button", { name: "Back" }).click();
  await expect(page.getByRole("heading", { name: "Scenario", exact: true })).toBeVisible();

  const snapshotPath = path.join(runDirectory, "data/cache/space-track/latest.json");
  const snapshot = JSON.parse(await readFile(snapshotPath, "utf8")) as {
    checksum: string;
    objects: unknown[];
    source: string;
  };
  expect(snapshot.checksum).not.toBe("");
  expect(snapshot.objects.length).toBeGreaterThan(0);

  if (useLiveProvider) {
    expect(snapshot.source).toContain("www.space-track.org/basicspacedata/query/class/gp/");
  } else {
    expect(mockLoginReceived).toBe(true);
    expect(mockCatalogDownloaded).toBe(true);
    expect(snapshot.objects).toHaveLength(2);
  }
});
