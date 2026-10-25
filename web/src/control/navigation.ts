import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

import type { Axes } from './jog'

/** Where the navigation preferences are kept, in this browser. */
export const NAVIGATION_KEY = 'spectrumiq.navigation'

/** How far a step goes, in µm: in x and y, on the stage's coarse z, and a notch of the focus. */
export interface Steps {
  xy: number
  z: number
  focus: number
}

export const DEFAULT_STEPS: Steps = { xy: 100, z: 50, focus: 1 }

/** The XY steps offered, in µm: fine, medium and coarse. */
export const XY_PRESETS = [10, 100, 1000] as const

export interface Inversion {
  x: boolean
  y: boolean
}

interface Saved {
  steps: Steps
  invert: Inversion
}

/** The saved preferences, keeping the defaults for anything missing or invalid. */
function read(): Saved {
  const saved: Saved = { steps: { ...DEFAULT_STEPS }, invert: { x: false, y: false } }
  try {
    const parsed = JSON.parse(window.localStorage.getItem(NAVIGATION_KEY) ?? '{}')
    for (const axis of ['xy', 'z', 'focus'] as const) {
      const step = parsed?.steps?.[axis]
      if (Number.isFinite(step) && step > 0) saved.steps[axis] = step
    }
    for (const axis of ['x', 'y'] as const) {
      if (typeof parsed?.invert?.[axis] === 'boolean') saved.invert[axis] = parsed.invert[axis]
    }
  } catch {
    // Unreadable, or no storage: the defaults.
  }
  return saved
}

/**
 * How the stage is navigated, kept in this browser: how far a step goes, and which of x and y
 * move the other way, for set-ups whose camera sees the stage turned or mirrored. The inversion
 * applies to click-to-move too.
 */
export const useNavigationPreferences = defineStore('navigation', () => {
  const saved = read()
  const steps = ref<Steps>(saved.steps)
  const invert = ref<Inversion>(saved.invert)
  watch(
    [steps, invert],
    () => {
      try {
        const value = { steps: steps.value, invert: invert.value }
        window.localStorage.setItem(NAVIGATION_KEY, JSON.stringify(value))
      } catch {
        // Without storage, the preferences last until the page reloads.
      }
    },
    { deep: true },
  )

  /** The signs of x and y: -1 where inverted. */
  function signs() {
    return { x: invert.value.x ? -1 : 1, y: invert.value.y ? -1 : 1 }
  }

  /** A step on each of the stage's axes, signed by the inversion: what a press towards + moves. */
  function signedSteps(): Axes {
    const { x, y } = signs()
    return { x: steps.value.xy * x, y: steps.value.xy * y, z: steps.value.z }
  }

  return { steps, invert, signs, signedSteps }
})
