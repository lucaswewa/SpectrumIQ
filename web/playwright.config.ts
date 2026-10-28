import { defineConfig, devices } from '@playwright/test'

import { CONFIG, freshInstrument, PORT, ROOT } from './e2e/instrument'

// The workers load this file too; only the runner, which starts the server, writes the instrument.
if (process.env.TEST_WORKER_INDEX === undefined) freshInstrument()

// The end-to-end tests (ADR-0069, conventions §5.3) run against the app as the server serves it:
// a debug build of this checkout, on a fresh simulated instrument, serving the fresh web build.
// They share the one instrument, so they run one at a time, in the installed Edge.
export default defineConfig({
  testDir: './e2e',
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: 'list',
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: 'retain-on-failure',
    viewport: { width: 1400, height: 900 },
  },
  projects: [
    // The users, the calibrations, so that the wizard stays shut, and a dataset to view.
    { name: 'setup', testMatch: /setup\.ts$/ },
    {
      name: 'edge',
      use: {
        ...devices['Desktop Edge'],
        channel: 'msedge',
        viewport: { width: 1400, height: 900 },
      },
      dependencies: ['setup'],
    },
  ],
  webServer: {
    command: `npm --prefix web run build && cargo run -q -p spectrumiq-server -- -c "${CONFIG}" --port ${PORT} --webapp-dir web/dist`,
    cwd: ROOT,
    url: `http://127.0.0.1:${PORT}/api/siq/system/health`,
    reuseExistingServer: false,
    // The first run builds the server.
    timeout: 600_000,
  },
})
