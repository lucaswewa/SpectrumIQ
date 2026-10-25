import './theme/tokens.css'
import './theme/base.css'

import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from './App.vue'
import { hostKey } from './host'
import { createBrowserHost } from './host/browser'
import { createAppRouter } from './router'
import { useThemeStore } from './theme/store'

const app = createApp(App)
app.use(createPinia())
// The bootstrap script in index.html applied the theme; the store keeps it in step from now on.
useThemeStore()
app.use(createAppRouter())
app.provide(hostKey, createBrowserHost())
app.mount('#app')
