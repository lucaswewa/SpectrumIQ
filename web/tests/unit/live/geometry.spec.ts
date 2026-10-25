// REQ-PRV-002: the ROI's geometry between the overview, the frame and the sensor (ADR-0038).
import { describe, expect, it } from 'vitest'

import {
  boxToFrame,
  dragged,
  frameSize,
  frameToBox,
  frameToSensor,
  roiArea,
  sensorToFrame,
  type Readout,
} from '@/live/geometry'

/** Sensor columns 1000.. and rows 500.., binned 2 × 2: 2000 × 1000 frame pixels. */
const binned: Readout = {
  region: { offset_x: 1000, offset_y: 500, width: 4000, height: 2000 },
  binning: { horizontal: 2, vertical: 2 },
}

describe('the ROI’s geometry', () => {
  it('sizes the frame from the region and the binning', () => {
    expect(frameSize(binned)).toEqual({ width: 2000, height: 1000 })
  })

  it('maps a point of the overview to the frame and back, across the letterboxing', () => {
    // A 2000 × 1000 frame, shown as a 1440 × 720 overview, in a 1000 × 1000 box: 250 px above.
    const [box, image, frame] = [
      { width: 1000, height: 1000 },
      { width: 1440, height: 720 },
      { width: 2000, height: 1000 },
    ]
    expect(boxToFrame({ x: 500, y: 500 }, box, image, frame)).toEqual({ x: 1000, y: 500 })
    expect(boxToFrame({ x: 250, y: 250 }, box, image, frame)).toEqual({ x: 500, y: 0 })
    // Off the image, on the letterboxing: clamped to the frame's edge.
    expect(boxToFrame({ x: 1200, y: 100 }, box, image, frame)).toEqual({ x: 2000, y: 0 })
    const rect = { x: 500, y: 250, width: 200, height: 100 }
    expect(frameToBox(rect, box, image, frame)).toEqual({ x: 250, y: 375, width: 100, height: 50 })
  })

  it('takes a dragged rectangle, either way round, to sensor pixels through the binning', () => {
    const [box, image, frame] = [
      { width: 2000, height: 1000 },
      { width: 2000, height: 1000 },
      frameSize(binned),
    ]
    const area = dragged({ x: 700, y: 350 }, { x: 500, y: 250 }, box, image, frame)
    expect(area).toEqual({ x: 500, y: 250, width: 200, height: 100 })
    expect(frameToSensor(area, binned)).toEqual({ x: 2000, y: 1000, width: 400, height: 200 })
    expect(sensorToFrame({ x: 2000, y: 1000, width: 400, height: 200 }, binned)).toEqual(area)
  })

  // The cases of siq-preview's `a_roi_maps_through_the_frame_s_region_and_binning_and_stays_inside_it`.
  it('shows the part of the frame that siq-preview’s ROI stream shows', () => {
    const frame = frameSize(binned)
    const shows = (x: number, y: number, width: number, height: number, zoom: 1 | 2) =>
      roiArea(frame, sensorToFrame({ x, y, width, height }, binned), zoom)
    expect(shows(2000, 1000, 400, 200, 1)).toEqual({ x: 500, y: 250, width: 200, height: 100 })
    // Half off the frame's left edge: what is inside.
    expect(shows(0, 1000, 1400, 200, 1)).toEqual({ x: 0, y: 250, width: 200, height: 100 })
    // Too large: cut to the most shown, about its middle, and at 2:1 twice as much.
    expect(shows(1000, 500, 4000, 2000, 1)).toEqual({ x: 40, y: 0, width: 1920, height: 1000 })
    expect(shows(1000, 500, 4000, 2000, 2)).toEqual({ x: 0, y: 0, width: 2000, height: 1000 })
    // Nothing of it in the frame, or nothing asked: the middle of the frame.
    const middle = { x: 40, y: 0, width: 1920, height: 1000 }
    expect(shows(0, 0, 500, 500, 1)).toEqual(middle)
    expect(roiArea(frame, undefined, 1)).toEqual(middle)
  })
})
