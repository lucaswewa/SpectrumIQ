import { defineStore } from 'pinia'
import { computed } from 'vue'

import { useThingMirror } from './thing'

/** The channels, and the one in use (ADR-0035). */
export const useImagingStore = defineStore('imaging', () => {
  const imaging = useThingMirror('imaging')
  const channel = imaging.property('channel')
  const channels = imaging.property('channels')
  const current = computed(() => channels.value?.find((each) => each.id === channel.value))

  return {
    fitted: imaging.fitted,
    channel,
    channels,
    current,
    /** Applies channel `id`: every light off, the filter, the exposure and colour, the light. */
    selectChannel: (id: number) => imaging.run('select_channel', { id }),
  }
})
