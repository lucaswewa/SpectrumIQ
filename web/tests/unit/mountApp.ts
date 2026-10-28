import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createMemoryHistory } from 'vue-router'

import App from '@/App.vue'
import { useSessionStore } from '@/auth/session'
import { hostKey } from '@/host'
import { createBrowserHost } from '@/host/browser'
import { createAppRouter } from '@/router'

/**
 * Mounts the whole app at `path`, attached to the document so it gets key presses, as against a
 * server that asks for no login: a test may log someone in after.
 */
export async function mountApp(path = '/', provide: Record<symbol, unknown> = {}) {
  const pinia = createPinia()
  setActivePinia(pinia)
  useSessionStore().checked = true
  const router = createAppRouter(createMemoryHistory())
  await router.push(path)
  await router.isReady()
  const wrapper = mount(App, {
    attachTo: document.body,
    global: {
      plugins: [pinia, router],
      provide: { [hostKey as symbol]: createBrowserHost(), ...provide },
    },
  })
  await flushPromises()
  return { wrapper, router }
}
