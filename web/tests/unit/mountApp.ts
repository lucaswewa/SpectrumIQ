import { flushPromises, mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { createMemoryHistory } from 'vue-router'

import App from '@/App.vue'
import { hostKey } from '@/host'
import { createBrowserHost } from '@/host/browser'
import { createAppRouter } from '@/router'

/** Mounts the whole app at `path`, attached to the document so it gets key presses. */
export async function mountApp(path = '/', provide: Record<symbol, unknown> = {}) {
  const router = createAppRouter(createMemoryHistory())
  await router.push(path)
  await router.isReady()
  const wrapper = mount(App, {
    attachTo: document.body,
    global: {
      plugins: [createPinia(), router],
      provide: { [hostKey as symbol]: createBrowserHost(), ...provide },
    },
  })
  await flushPromises()
  return { wrapper, router }
}
