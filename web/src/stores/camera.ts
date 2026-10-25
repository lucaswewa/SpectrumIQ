import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import { useThingMirror, type EventData, type Value } from './thing'

/**
 * The camera: its settings, its statistics (polled each second: they aren't observable), the
 * frames it lost, and live view and snaps. Its streams' URLs come from its description.
 */
export const useCameraStore = defineStore('camera', () => {
  const camera = useThingMirror('camera')
  const exposureMs = camera.property('exposure_ms')
  const region = camera.property('region')
  const binning = camera.property('binning')
  const pixelFormat = camera.property('pixel_format')
  const frameRate = camera.property('frame_rate', { pollMs: 2000 })
  const sensor = camera.property('sensor')
  const stats = camera.property('stats', { pollMs: 1000 })
  const framesLost = shallowRef<EventData<'camera', 'frames_lost'>>()
  camera.on('frames_lost', (lost) => (framesLost.value = lost))
  const live = computed(() => stats.value?.mode === 'live')

  async function startLive() {
    await camera.run('start_live')
    await camera.read('stats', stats)
  }

  async function stopLive() {
    await camera.run('stop_live')
    await camera.read('stats', stats)
  }

  return {
    fitted: camera.fitted,
    exposureMs,
    region,
    binning,
    pixelFormat,
    frameRate,
    sensor,
    stats,
    framesLost,
    live,
    setExposure: (ms: number) => camera.write('exposure_ms', ms, exposureMs),
    setRegion: (next: Value<'camera', 'region'>) => camera.write('region', next, region),
    setBinning: (next: Value<'camera', 'binning'>) => camera.write('binning', next, binning),
    setPixelFormat: (next: Value<'camera', 'pixel_format'>) =>
      camera.write('pixel_format', next, pixelFormat),
    startLive,
    stopLive,
    /** Takes a frame once the stage and focus are still: its JPEG preview, as a link. */
    snap: (save = false) => camera.run('snap', { save }),
    /** The URL of stream `name`, `overview` or `roi`. */
    streamUrl: (name: 'overview' | 'roi') => camera.thing.value?.streamUrl(name),
  }
})
