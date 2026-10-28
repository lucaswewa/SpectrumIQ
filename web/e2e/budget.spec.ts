// REQ-NFR-003, REQ-SEC-002: the connection budget (ADR-0010, ADR-0069). One browser holds the 3
// MJPEG streams it allows, the overview, the ROI and the live mosaic's view, and the Things'
// observation sockets, while the viewer loads a dataset's deep-zoom tiles over the 3 connections
// left of the 6 HTTP/1.1 allows: every tile arrives, and the streams and observations go on.
import { expect, type Page, type Request } from '@playwright/test'

import { logIn, test, tokenBar } from './session'

const STREAM = /\/api\/v1\/(camera\/(overview|roi)|mosaic_live\/view)$/
const TILE = /\/api\/siq\/data\/.+_files\/\d+\/\d+_\d+\.jpeg$/

const drawn = async (page: Page, stream: string) =>
  Number(
    await page.locator(`.live-image[data-stream="${stream}"] canvas`).getAttribute('data-frames'),
  )

test('three streams, the observations and the tiles share the connections', async ({ browser }) => {
  const context = await browser.newContext()
  const streams = new Set<Request>()
  const tiles = { asked: new Set<Request>(), done: 0, failed: 0 }
  const observations = { sockets: new Set<string>(), messages: 0 }
  context.on('request', (request) => {
    const path = new URL(request.url()).pathname
    if (STREAM.test(path)) streams.add(request)
    if (TILE.test(path)) tiles.asked.add(request)
  })
  context.on('requestfinished', (request) => {
    streams.delete(request)
    if (tiles.asked.has(request)) tiles.done += 1
  })
  context.on('requestfailed', (request) => {
    streams.delete(request)
    // The viewer cancels the tiles it no longer needs as it zooms in: they didn't fail.
    const aborted = request.failure()?.errorText.includes('ERR_ABORTED')
    if (tiles.asked.has(request)) tiles[aborted ? 'done' : 'failed'] += 1
  })
  context.on('page', (page) =>
    page.on('websocket', (socket) => {
      observations.sockets.add(new URL(socket.url()).pathname)
      socket.on('framereceived', () => (observations.messages += 1))
    }),
  )

  const live = await context.newPage()
  await logIn(live, 'ada')
  await tokenBar(live).getByRole('button', { name: 'Take control' }).click()
  await live.getByRole('button', { name: 'Start live' }).first().click()
  await expect(live.locator('.live-image[data-status="live"]')).toHaveCount(2)
  const mosaic = await context.newPage()
  await mosaic.goto('/#/mosaic')
  await expect.poll(() => streams.size).toBe(3)

  const before = { overview: await drawn(live, 'overview'), messages: observations.messages }
  const viewer = await context.newPage()
  await viewer.goto('/#/viewer')
  await expect.poll(() => tiles.asked.size, { timeout: 20_000 }).toBeGreaterThan(8)
  await expect.poll(() => tiles.asked.size - tiles.done - tiles.failed, { timeout: 20_000 }).toBe(0)
  expect(tiles.failed).toBe(0)
  expect(streams.size).toBe(3)
  expect(await drawn(live, 'overview')).toBeGreaterThan(before.overview)
  expect(observations.messages).toBeGreaterThan(before.messages)
  expect([...observations.sockets].every((path) => /^\/api\/v1\/\w+\/ws$/.test(path))).toBe(true)

  await live.getByRole('button', { name: 'Stop live' }).first().click()
  await tokenBar(live).getByRole('button', { name: 'Release control' }).click()
  await expect(tokenBar(live)).toContainText('No one has control.')
  await context.close()
})
