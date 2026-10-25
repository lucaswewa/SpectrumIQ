import { readonly, ref } from 'vue'

const flashes = ref(0)

/** Flashes every live image, as a capture is taken (P26). */
export function flashLiveImage() {
  flashes.value += 1
}

/** How many flashes there have been: a live image flashes each time it grows. */
export function useCaptureFlashes() {
  return readonly(flashes)
}
