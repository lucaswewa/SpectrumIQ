// REQ-SEC-002: two browsers sharing the instrument (ADR-0066, ADR-0068): an operator's control
// seen by the others, an admin taking it over, its release; and a holder gone silent losing it
// once the token's timeout has passed.
import { expect } from '@playwright/test'

import { CONTROL_TIMEOUT_S } from './instrument'
import { logIn, test, tokenBar } from './session'

test('two browsers hand the token over', async ({ browser }) => {
  const [olga, ada] = await Promise.all([browser.newPage(), browser.newPage()])
  await logIn(olga, 'olga')
  await logIn(ada, 'ada')
  await tokenBar(olga).getByRole('button', { name: 'Take control' }).click()
  await expect(tokenBar(olga)).toContainText('You have control of the instrument.')
  await expect(tokenBar(ada)).toContainText(/olga has control since .+: you see what they do\./)
  await expect(ada.getByRole('button', { name: 'Snap' })).toBeDisabled()

  await tokenBar(ada).getByRole('button', { name: 'Take over' }).click()
  await expect(tokenBar(ada)).toContainText('You have control of the instrument.')
  await expect(ada.getByRole('button', { name: 'Snap' })).toBeEnabled()
  await expect(tokenBar(olga)).toContainText(/ada has control since/)
  await expect(olga.getByRole('button', { name: 'Snap' })).toBeDisabled()
  await expect(olga.getByText('Take control to change the instrument.').first()).toBeVisible()
  // An operator can't take it from another.
  await expect(tokenBar(olga).getByRole('button', { name: 'Take over' })).toHaveCount(0)

  await tokenBar(ada).getByRole('button', { name: 'Release control' }).click()
  await expect(tokenBar(olga)).toContainText('No one has control.')
  await Promise.all([olga.context().close(), ada.context().close()])
})

test('a holder gone silent loses the token', async ({ browser }) => {
  const [otto, vic] = await Promise.all([browser.newPage(), browser.newPage()])
  await logIn(otto, 'otto')
  await logIn(vic, 'vic')
  await tokenBar(otto).getByRole('button', { name: 'Take control' }).click()
  await expect(tokenBar(vic)).toContainText('otto has control')
  // Its browser closed, its heartbeat stops.
  await otto.context().close()
  await expect(tokenBar(vic)).toContainText('View only.', {
    timeout: (CONTROL_TIMEOUT_S + 10) * 1000,
  })
  await vic.context().close()
})
