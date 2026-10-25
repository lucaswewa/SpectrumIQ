import { defineStore } from 'pinia'

import { useThingMirror, type Value } from './thing'

/** How the previews show the frames: the mapping and the ROI, and the ROI's latest statistics. */
export const useDisplayStore = defineStore('display', () => {
  const display = useThingMirror('display')
  const lut = display.property('lut')
  const roiRegion = display.property('roi_region')
  const histogram = display.property('histogram', { pollMs: 500 })
  const focusMeasure = display.property('focus_measure', { pollMs: 500 })

  async function autoContrastOnce() {
    await display.run('auto_contrast_once')
    await display.read('lut', lut)
  }

  return {
    fitted: display.fitted,
    lut,
    roiRegion,
    histogram,
    focusMeasure,
    setLut: (next: Value<'display', 'lut'>) => display.write('lut', next, lut),
    setRoiRegion: (next: Value<'display', 'roi_region'>) =>
      display.write('roi_region', next, roiRegion),
    autoContrastOnce,
  }
})
