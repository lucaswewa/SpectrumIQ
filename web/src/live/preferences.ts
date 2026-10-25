import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

/** Where the "Disable stream" preference is kept, in this browser. */
export const STREAM_DISABLED_KEY = 'spectrumiq.stream-disabled'

function readDisabled(): boolean {
  try {
    return window.localStorage.getItem(STREAM_DISABLED_KEY) === 'true'
  } catch {
    return false
  }
}

/** Where the Live page's ROI view is: beside the overview, or in its corner. */
export const ROI_LAYOUT_KEY = 'spectrumiq.roi-layout'
export type RoiLayout = 'split' | 'pip'

function readLayout(): RoiLayout {
  try {
    return window.localStorage.getItem(ROI_LAYOUT_KEY) === 'pip' ? 'pip' : 'split'
  } catch {
    return 'split'
  }
}

function keep(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value)
  } catch {
    // Without storage, the choice lasts until the page reloads.
  }
}

/** The live image's preferences, kept in this browser. */
export const useLiveImagePreferences = defineStore('live-image', () => {
  /** Whether the live stream is off, to spare the network or the computer. */
  const streamDisabled = ref(readDisabled())
  watch(streamDisabled, (disabled) => keep(STREAM_DISABLED_KEY, String(disabled)))
  const roiLayout = ref<RoiLayout>(readLayout())
  watch(roiLayout, (layout) => keep(ROI_LAYOUT_KEY, layout))
  return { streamDisabled, roiLayout }
})
