import { defineStore } from 'pinia'

import { throttle } from '@/mosaic/throttle'
import type { Viewport } from '@/mosaic/viewport'

import { notify, useThingMirror } from './thing'

/** The least time between two viewports written, in ms: the server renders 10 views a second. */
export const VIEWPORT_MS = 100

/**
 * The live mosaic (ADR-0059): whether it paints, what was painted and at what scale, how it
 * goes, and the viewport the server renders, written at most every 100 ms. `viewport` is the one
 * the server last took: what the stream's images show.
 */
export const useMosaicStore = defineStore('mosaic', () => {
  const mosaic = useThingMirror('mosaic_live')
  const enabled = mosaic.property('enabled')
  const viewport = mosaic.property('viewport')
  const bounds = mosaic.property('bounds', { pollMs: 1_000 })
  const pxPerUm = mosaic.property('px_per_um', { pollMs: 1_000 })
  const stats = mosaic.property('stats', { pollMs: 2_000 })
  const setViewport = throttle((next: Viewport) => {
    void mosaic.write('viewport', next).then((took) => {
      if (took) viewport.value = next
    })
  }, VIEWPORT_MS)

  async function clear() {
    await mosaic.run('clear')
    await mosaic.read('bounds', bounds)
  }

  /** Saves what was painted as a dataset, and says so. */
  async function save() {
    const saved = await mosaic.run('save_snapshot', {})
    if (saved) notify('success', `The mosaic was saved as the dataset "${saved.dataset.name}".`)
    return saved
  }

  return {
    fitted: mosaic.fitted,
    enabled,
    viewport,
    bounds,
    pxPerUm,
    stats,
    setEnabled: (on: boolean) => mosaic.write('enabled', on, enabled),
    setViewport,
    clear,
    save,
  }
})
