import { fileURLToPath } from 'node:url'

import { defineConfig, devices } from '@playwright/test'

const port = 5098
const root = fileURLToPath(new URL('..', import.meta.url))

// The end-to-end tests (from M9, conventions §5.3) run against the app as the server serves it:
// a debug build of this checkout, on the simulated instrument, serving the fresh web build.
export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  use: { baseURL: `http://127.0.0.1:${port}`, trace: 'on-first-retry' },
  projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: `npm --prefix web run build && cargo run -q -p spectrumiq-server -- -c config/sim.toml --port ${port} --webapp-dir web/dist`,
    cwd: root,
    url: `http://127.0.0.1:${port}/api/siq/system/health`,
    reuseExistingServer: !process.env.CI,
    timeout: 600_000,
  },
})
