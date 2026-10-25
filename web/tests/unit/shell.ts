import { defineComponent, h, type Component } from 'vue'

import { provideShortcuts } from '@/app/shortcuts'
import OverlayProvider from '@/ui/OverlayProvider.vue'

/** `page`, with what the app shell gives a page: the shortcut registry, confirmations and toasts. */
export function inShell(page: Component) {
  return defineComponent(() => {
    provideShortcuts()
    return () => h(OverlayProvider, null, { default: () => h(page) })
  })
}
