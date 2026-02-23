import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  timeout: 120_000,
  workers: 1,
  reporter: "html",

  use: {
    baseURL: "http://localhost:5174",
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    video: "retain-on-failure",
  },

  webServer: {
    command: "pnpm run dev -- --port 5174",
    url: "http://localhost:5174",
    reuseExistingServer: !process.env.CI,
    env: {
      VITE_ROLLUP_URL: process.env.VITE_ROLLUP_URL || "http://localhost:12346",
      VITE_CHAIN_ID: process.env.VITE_CHAIN_ID || "4321",
      VITE_SOLANA_ENDPOINT:
        process.env.VITE_SOLANA_ENDPOINT ||
        "/sequencer/accept-solana-offchain-tx",
    },
  },
});
