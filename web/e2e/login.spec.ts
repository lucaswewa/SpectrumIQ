// REQ-SEC-001, REQ-SEC-002: the login in front of the app, a wrong password refused, a viewer's
// controls disabled with the reason, and a logout asking again (ADR-0068).
import { expect } from '@playwright/test'

import { PASSWORD } from './instrument'
import { logIn, test, tokenBar } from './session'

test('the app asks for a login, refuses a wrong password, and asks again after a logout', async ({
  page,
}) => {
  await page.goto('/#/live')
  const form = page.getByRole('form', { name: 'Log in' })
  await form.getByLabel('Name').fill('vic')
  await form.getByLabel('Password').fill(`${PASSWORD}!`)
  await form.getByRole('button', { name: 'Log in' }).click()
  await expect(form).toContainText('the name or the password is wrong')

  await logIn(page, 'vic')
  await expect(tokenBar(page)).toContainText('View only.')
  await expect(page.getByText('View only: your role is viewer.').first()).toBeVisible()
  await expect(page.getByRole('button', { name: 'Snap' })).toBeDisabled()
  await expect(page.getByRole('button', { name: 'STOP', exact: true })).toBeDisabled()

  await tokenBar(page).getByRole('button', { name: 'Log out' }).click()
  await expect(page.getByRole('form', { name: 'Log in' })).toBeVisible()
})
