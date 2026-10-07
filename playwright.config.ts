import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  testMatch: /.*\.spec\.ts$/,
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  // One retry absorbs transient local dev-server hiccups (a dropped keep-alive
  // connection, a slow Vite reload). A real defect still fails on the retry.
  retries: 1,
  expect: { timeout: 10_000 },
  reporter: "line",
  use: {
    baseURL: "http://127.0.0.1:5173",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  webServer: [
    {
      command:
        ".venv\\Scripts\\python.exe -m uvicorn backend.app:app --host 127.0.0.1 --port 8000",
      url: "http://127.0.0.1:8000/api/health",
      env: {
        OUTLOOK_NEXT_DB: "data/e2e.db",
        OUTLOOK_NEXT_DISABLE_COPILOT_CLI: "1",
        OUTLOOK_NEXT_CALENDAR_CACHE: "data/e2e-calendar-cache.json",
      },
      reuseExistingServer: true,
      timeout: 120_000,
    },
    {
      command: "npm run dev",
      url: "http://127.0.0.1:5173",
      reuseExistingServer: true,
      timeout: 120_000,
    },
  ],
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    // Mobile viewport on Chromium: the same engine as the desktop project, so a
    // single browser download covers both and no engine-specific noise appears.
    { name: "mobile", use: { ...devices["Pixel 5"] } },
  ],
});
