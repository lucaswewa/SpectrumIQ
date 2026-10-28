// REQ-SEC-002, REQ-VIS-005: an operator takes control, and live view streams both previews to the
// Live page; then it stops, and the token goes back (ADR-0068).
import { expect, type Page } from '@playwright/test'

import { logIn, test, tokenBar } from './session'

/** The frames the Live page's `stream` has drawn. */
const drawn = async (page: Page, stream: 'overview' | 'roi') =>
  Number(
    await page.locator(`.live-image[data-stream="${stream}"] canvas`).getAttribute('data-frames'),
  )

test('an operator takes control, and live view streams both previews', async ({ page }) => {
  await logIn(page, 'olga')
  await expect(page.getByRole('button', { name: 'Snap' })).toBeDisabled()
  await tokenBar(page).getByRole('button', { name: 'Take control' }).click()
  await expect(tokenBar(page)).toContainText('You have control of the instrument.')
  // Calibrated by the setup: no wizard.
  await expect(page.getByRole('dialog')).toHaveCount(0)

  await page.getByRole('button', { name: 'Start live' }).first().click()
  for (const stream of ['overview', 'roi'] as const) {
    await expect(page.locator(`.live-image[data-stream="${stream}"]`)).toHaveAttribute(
      'data-status',
      'live',
    )
    const before = await drawn(page, stream)
    await expect.poll(() => drawn(page, stream)).toBeGreaterThan(before + 2)
  }

  await page.getByRole('button', { name: 'Stop live' }).first().click()
  await expect(page.getByRole('button', { name: 'Start live' }).first()).toBeVisible()
  await tokenBar(page).getByRole('button', { name: 'Release control' }).click()
  await expect(tokenBar(page)).toContainText('No one has control.')
})
