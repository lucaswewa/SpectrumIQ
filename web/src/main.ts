import './theme/tokens.css'
import './theme/base.css'

import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from './App.vue'
import { thingAvailabilityKey } from './app/availability'
import { hostKey } from './host'
import { createBrowserHost } from './host/browser'
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
app.provide(hostKey, createBrowserHost())
app.mount('#app')
void connection.start()
