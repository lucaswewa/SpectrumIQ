// The mapping of each channel of the image in the viewer (ADR-0009, amended): `channels`, which
// the controls change at once, and `applied`, which the tiles are fetched with. A change to the
// mapping is applied once the controls rest for SETTLE_MS, so that a slider being dragged fetches
// tiles only where it stops; showing or hiding a channel is applied at once.
import { defineStore } from 'pinia'
import { ref, shallowRef } from 'vue'

import type { ChannelLut, ImageChannel } from './tiles'

/** A channel's mapping, and whether it is shown. */
export interface ChannelView extends ChannelLut {
  label: string
  visible: boolean
}

/** How long the controls rest before a new mapping is fetched. */
export const SETTLE_MS = 250
/** The gamma's range, as the tile routes take it. */
export const GAMMA: [number, number] = [0.05, 20]

/** A channel as its image describes it: its own window and colour, shown. */
const own = (channel: ImageChannel): ChannelView => ({
  label: channel.label,
  min: channel.window[0],
  max: channel.window[1],
  gamma: 1,
  colour: channel.colour,
  visible: true,
})

/** `view` made one the routes take: a gamma in range, and a window that isn't empty. */
function valid(view: ChannelView): ChannelView {
  const gamma = Math.min(GAMMA[1], Math.max(GAMMA[0], view.gamma))
  const max = view.max > view.min ? view.max : view.min + 1
  return { ...view, gamma, max }
}

export const useViewerLuts = defineStore('viewer-luts', () => {
  const channels = ref<ChannelView[]>([])
  const applied = shallowRef<ChannelView[]>([])
  let image: ImageChannel[] = []
  let timer: ReturnType<typeof setTimeout> | undefined

  /** Applies the controls' mapping now. */
  function apply() {
    clearTimeout(timer)
    applied.value = channels.value.map(valid)
  }

  /** Starts again from `channels`, an image's: their own windows and colours, all shown. */
  function load(next: ImageChannel[]) {
    image = next
    channels.value = next.map(own)
    apply()
  }

  /** Changes channel `c`: its visibility at once, its mapping once the controls rest. */
  function set(c: number, change: Partial<ChannelView>) {
    const channel = channels.value[c]
    if (!channel) return
    Object.assign(channel, change)
    if (Object.keys(change).every((key) => key === 'visible')) {
      apply()
    } else {
      clearTimeout(timer)
      timer = setTimeout(apply, SETTLE_MS)
    }
  }

  /** Gives channel `c` its own mapping again. */
  function reset(c: number) {
    const channel = image[c]
    if (!channel) return
    channels.value[c] = { ...own(channel), visible: channels.value[c]?.visible ?? true }
    apply()
  }

  return { channels, applied, load, set, reset, apply }
})
