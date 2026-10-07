import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: { "/api": "http://127.0.0.1:8000" },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test-setup.ts",
    css: true,
    globals: true,
    // Vitest owns *.test.ts(x); Playwright owns *.spec.ts. Keeping them apart
    // stops a stray Playwright file from failing the unit test run.
    include: ["src/**/*.test.{ts,tsx}"],
    exclude: ["e2e/**", "node_modules/**", "dist/**", "**/*.spec.ts"],
    testTimeout: 15_000,
    hookTimeout: 15_000,
  },
});
