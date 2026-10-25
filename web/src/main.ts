import './theme/tokens.css'
import './theme/base.css'

import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from './App.vue'
import { thingAvailabilityKey } from './app/availability'
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
// A browser's host, or the desktop shell's (ADR-0040), which asks the shell first.
void createHost().then((host) => {
  app.provide(hostKey, host)
  app.mount('#app')
  void connection.start()
})
