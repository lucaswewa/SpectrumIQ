import type { Component } from 'vue'
import {
  createRouter,
  createWebHashHistory,
  type RouteRecordRaw,
  type RouterHistory,
} from 'vue-router'

import PlannedPage from '@/app/PlannedPage.vue'
import { DESTINATIONS } from '@/app/navigation'
import AboutPage from '@/system/AboutPage.vue'

/** The pages that are built; every other destination shows what will come (`PlannedPage`). */
const PAGES: Record<string, Component> = {
  about: AboutPage,
}

/** One route per destination in the rail. */
const destinationRoutes: RouteRecordRaw[] = DESTINATIONS.map((destination) => ({
  path: `/${destination.id}`,
  name: destination.id,
  component: PAGES[destination.id] ?? PlannedPage,
  props: PAGES[destination.id] ? false : { destination: destination.id },
}))

/** Pages for development only: left out of production builds. */
const devRoutes: RouteRecordRaw[] = import.meta.env.DEV
  ? [
      {
        path: '/dev/tokens',
        name: 'dev-tokens',
        component: () => import('@/views/dev/TokensView.vue'),
      },
      {
        path: '/dev/components',
        name: 'dev-components',
        component: () => import('@/views/dev/ComponentsView.vue'),
      },
    ]
  : []

/**
 * The app's router. Routes live in the URL's hash (`/#/live`), which
 * works however the app is served: by spectrumiq-server, by Vite, or
 * in the Tauri shell (ADR-0008).
 */
export function createAppRouter(history: RouterHistory = createWebHashHistory()) {
  return createRouter({
    history,
    routes: [
      { path: '/', redirect: { name: 'live' } },
      ...destinationRoutes,
      ...devRoutes,
      { path: '/:unknown(.*)*', redirect: { name: 'live' } },
    ],
  })
}
