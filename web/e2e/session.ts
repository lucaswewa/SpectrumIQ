// The tests' start, with the token free and live view stopped; logging in as a test user through
// the login page; and the control token's bar.
import { test as base, expect, type Page } from '@playwright/test'

import { PASSWORD } from './instrument'

/** The control token's bar, above every page. */
export const tokenBar = (page: Page) =>
  page.getByRole('status', { name: 'Control of the instrument' })

/**
 * The tests, each from the same start: whatever a test before left, an admin takes the token over,
 * stops live view and lets the token go. The browsers a test opens itself are closed after it.
 */
export const test = base.extend<{ fresh: void }>({
  fresh: [
    async ({ request, browser, context }, use) => {
      await request.post('/api/siq/auth/login', { data: { name: 'ada', password: PASSWORD } })
      await request.post('/api/siq/control/take_over')
      await request.post('/api/v1/camera/stop_live')
      expect((await request.post('/api/siq/control/release')).ok()).toBe(true)
      await use()
      const opened = browser.contexts().filter((each) => each !== context)
      await Promise.all(opened.map((each) => each.close()))
    },
    { auto: true },
  ],
})

/** Opens `path`, and logs in as `name` on the login page in front of it. */
export async function logIn(page: Page, name: string, path = '/#/live') {
  await page.goto(path)
  await page.getByLabel('Name').fill(name)
  await page.getByLabel('Password').fill(PASSWORD)
  await page.getByRole('button', { name: 'Log in' }).click()
  await expect(tokenBar(page)).toContainText(`${name},`)
}
