// The instrument's preparation, through the API: the users (the first admin with the CLI, the rest
// by it over HTTP); the stage–camera calibration and every channel's dark and flat, so that the
// calibration wizard stays shut; and a snap into a dataset, with its pyramid, for the viewer.
import { execFileSync } from 'node:child_process'

import { expect, test as setup, type APIRequestContext } from '@playwright/test'

import { CONFIG, PASSWORD, ROOT } from './instrument'

/** Runs `action` (`thing/action`) through the API, and waits for its output. */
async function run(request: APIRequestContext, action: string, data: object = {}) {
  let invocation = await (await request.post(`/api/v1/${action}`, { data })).json()
  while (['pending', 'running'].includes(invocation.status)) {
    await new Promise((resolve) => setTimeout(resolve, 200))
    invocation = await (await request.get(new URL(invocation.href).pathname)).json()
  }
  expect(invocation.status, `${action}: ${JSON.stringify(invocation)}`).toBe('completed')
  return invocation.output
}

setup('the users, the calibrations and a dataset', async ({ request }) => {
  setup.setTimeout(300_000)
  const add = ['run', '-q', '-p', 'spectrumiq-server', '--', 'user', 'add', 'ada', '-c', CONFIG]
  execFileSync('cargo', add, { cwd: ROOT, input: `${PASSWORD}\n${PASSWORD}\n` })
  const login = await request.post('/api/siq/auth/login', {
    data: { name: 'ada', password: PASSWORD },
  })
  expect(login.ok()).toBe(true)
  for (const [name, role] of [
    ['olga', 'operator'],
    ['otto', 'operator'],
    ['vic', 'viewer'],
  ]) {
    const added = await request.post('/api/siq/users', { data: { name, password: PASSWORD, role } })
    expect(added.status()).toBe(201)
  }

  expect((await request.post('/api/siq/control/request')).ok()).toBe(true)
  await run(request, 'calibration/stage_camera')
  const channels: { id: number }[] = await (await request.get('/api/v1/imaging/channels')).json()
  for (const { id } of channels) {
    await run(request, 'imaging/select_channel', { id })
    await run(request, 'calibration/capture_dark', { frames: 2 })
    await run(request, 'calibration/capture_flat', { frames: 9 })
  }
  const snap = await run(request, 'datasets/snap_to_dataset')
  await expect
    .poll(
      async () => {
        const manifest = await (
          await request.get(`/api/siq/data/${snap.dataset.id}/manifest`)
        ).json()
        return manifest.history.findLast((record: { kind: string }) => record.kind === 'pyramid')
          ?.state
      },
      { timeout: 60_000 },
    )
    .toBe('done')
  await run(request, 'imaging/select_channel', { id: channels[0]!.id })
  await run(request, 'camera/stop_live')
  expect((await request.post('/api/siq/control/release')).ok()).toBe(true)
})
