import './theme/tokens.css'
import './theme/base.css'

import { createPinia } from 'pinia'
import { createApp, watch } from 'vue'

import App from './App.vue'
import { thingAvailabilityKey } from './app/availability'
import { useSessionStore } from './auth/session'
import { createHost, hostKey } from './host'
import { createAppRouter } from './router'
import { useConnectionStore } from './stores/connection'
import { useThemeStore } from './theme/store'

const app = createApp(App)
app.use(createPinia())
// The bootstrap script in index.html applied the theme; the store keeps it in step from now on.
useThemeStore()
// Destinations show only when the server has the Things they need.
const connection = useConnectionStore()
app.provide(thingAvailabilityKey, (thing) => connection.isAvailable(thing))
app.use(createAppRouter())
// A browser's host, or the desktop shell's (ADR-0040), which asks the shell first. The session
// comes first, by the local secret on the instrument's computer; the connection follows it. The
// development pages bring their own fake instrument, without a login.
const session = useSessionStore()
void createHost().then(async (host) => {
  app.provide(hostKey, host)
  if (import.meta.env.DEV && location.hash.startsWith('#/dev/')) session.checked = true
  else await session.start(host)
  app.mount('#app')
  watch(
    () => session.admitted,
    (admitted) => (admitted ? void connection.start() : connection.stop()),
    { immediate: true },
  )
})
