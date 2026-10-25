// Which overlay layers are shown (ADR-0009, amended), kept in this browser by each
// layer's id. A layer not chosen yet is shown.
import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

import type { OverlayLayer } from './types'

const LAYERS_KEY = 'spectrumiq.overlay-layers'

function read(): Record<string, boolean> {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(LAYERS_KEY) ?? '{}')
    return Object.fromEntries(
      Object.entries(parsed ?? {}).filter(([, shown]) => typeof shown === 'boolean'),
    ) as Record<string, boolean>
  } catch {
    // Unreadable, or no storage: every layer shown.
    return {}
  }
}

export const useOverlayLayers = defineStore('overlay-layers', () => {
  const chosen = ref(read())
  watch(
    chosen,
    (value) => {
      try {
        window.localStorage.setItem(LAYERS_KEY, JSON.stringify(value))
      } catch {
        // Without storage, the choices last until the page reloads.
      }
    },
    { deep: true },
  )

  const shown = (id: string) => chosen.value[id] ?? true
  /** Shows or hides layer `id`. */
  const show = (id: string, on: boolean) => (chosen.value[id] = on)
  /** Those of `layers` that are shown. */
  const visible = (layers: OverlayLayer[]) => layers.filter((layer) => shown(layer.id))

  return { shown, show, visible }
})
