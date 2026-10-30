import type { Component } from 'vue'
import {
  createRouter,
  createWebHashHistory,
  type RouteRecordRaw,
  type RouterHistory,
} from 'vue-router'

import PlannedPage from '@/app/PlannedPage.vue'
import CalibrationPage from '@/calibration/CalibrationPage.vue'
import { DESTINATIONS } from '@/app/navigation'
import DatasetsPage from '@/datasets/DatasetsPage.vue'
import JobsPage from '@/jobs/JobsPage.vue'
import LivePage from '@/live/LivePage.vue'
import MosaicPage from '@/mosaic/MosaicPage.vue'
import ScanPage from '@/scan/ScanPage.vue'
import AboutPage from '@/system/AboutPage.vue'
import SettingsPage from '@/system/SettingsPage.vue'
import ViewerPage from '@/viewer2d/ViewerPage.vue'

/** The pages that are built; every other destination shows what will come (`PlannedPage`). */
const PAGES: Record<string, Component | (() => Promise<{ default: Component }>)> = {
  about: AboutPage,
  // An admin's alone, with its table: loaded when first opened.
  admin: () => import('@/admin/AdminPage.vue'),
  calibration: CalibrationPage,
  datasets: DatasetsPage,
  jobs: JobsPage,
  live: LivePage,
  mosaic: MosaicPage,
  scan: ScanPage,
  settings: SettingsPage,
  viewer: ViewerPage,
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
      {
        path: '/dev/observe',
        name: 'dev-observe',
        component: () => import('@/views/dev/ObserveView.vue'),
      },
      {
        path: '/dev/live',
        name: 'dev-live',
        component: () => import('@/views/dev/LiveDemoView.vue'),
      },
      {
        path: '/dev/control',
        name: 'dev-control',
        component: () => import('@/views/dev/ControlDemoView.vue'),
      },
      {
        path: '/dev/viewer2d',
        name: 'dev-viewer2d',
        component: () => import('@/views/dev/Viewer2DDemoView.vue'),
      },
      {
        path: '/dev/calibration',
        name: 'dev-calibration',
        component: () => import('@/views/dev/CalibrationDemoView.vue'),
      },
      {
        path: '/dev/scan',
        name: 'dev-scan',
        component: () => import('@/views/dev/ScanDemoView.vue'),
      },
      {
        path: '/dev/run',
        name: 'dev-run',
        component: () => import('@/views/dev/RunDemoView.vue'),
      },
      {
        path: '/dev/mosaic',
        name: 'dev-mosaic',
        component: () => import('@/views/dev/MosaicDemoView.vue'),
      },
      {
        path: '/dev/admin',
        name: 'dev-admin',
        component: () => import('@/views/dev/AdminDemoView.vue'),
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
