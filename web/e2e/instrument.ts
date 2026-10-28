// The instrument the end-to-end tests run against (ADR-0069): `config/sim.toml` with a 2048 ×
// 1536 camera, rendered fast, and a 15 s token timeout, in a folder of its own emptied for each
// run, so that the tests start from a fresh instrument and leave the repository's data alone.
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/** The repository's root. */
export const ROOT = fileURLToPath(new URL('../..', import.meta.url))
/** The port the server under test listens on. */
export const PORT = 5098
/** Every test user's password. */
export const PASSWORD = 'correct horse'
/** The token's timeout, in seconds: a third of a minute, so that a test can wait it out. */
export const CONTROL_TIMEOUT_S = 15

const FOLDER = path.join(tmpdir(), 'spectrumiq-e2e')
/** The configuration's path. Its data, settings, audit and logs are beside it, in `FOLDER`. */
export const CONFIG = path.join(FOLDER, 'config', 'sim.toml')

/** Writes the configuration, in a folder emptied first. */
export function freshInstrument() {
  rmSync(FOLDER, { recursive: true, force: true })
  mkdirSync(path.dirname(CONFIG), { recursive: true })
  const toml = readFileSync(path.join(ROOT, 'config', 'sim.toml'), 'utf8')
    .replace(/^width = \d+/m, 'width = 2048')
    .replace(/^height = \d+/m, 'height = 1536')
    .replace('profile = "full"', 'profile = "fast"')
    .replace(/^\[auth\].*$/m, `$&\ncontrol_timeout_s = ${CONTROL_TIMEOUT_S}`)
  writeFileSync(CONFIG, toml)
}
