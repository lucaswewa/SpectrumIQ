import type { Point } from '@/live/contain'
import { useCameraStore } from '@/stores/camera'
import { useStageStore } from '@/stores/stage'
import { usePeripheralsStore } from '@/stores/peripherals'
import type { Value } from '@/stores/thing'

import { stageWithinTravel } from './jog'
import { useNavigationPreferences } from './navigation'

/** The magnification assumed when the server has no objective in place. */
export const NOMINAL_MAGNIFICATION = 10

export interface Optics {
  /** The sensor region read out, in sensor pixels: what the live image shows. */
  region: Value<'camera', 'region'>
  /** The sensor's pixel pitch, in µm. */
  pixelSize: number
  magnification: number
  /** -1 on an axis the navigation preferences invert. */
  signs: { x: number; y: number }
}

/**
 * The stage move, in µm, that brings `point` of the live image to its centre: the nominal
 * mapping until calibration (M5). `point` is a share of the image's width and height from its
 * centre, rightwards and downwards. The point shows the sample where the stage would be at
 * its position plus the move: the image's x and y are the stage's (ADR-0039).
 */
export function clickMove(point: Point, optics: Optics): { dx: number; dy: number } {
  const umPerPixel = optics.pixelSize / optics.magnification
  return {
    dx: point.x * optics.region.width * umPerPixel * optics.signs.x,
    dy: point.y * optics.region.height * umPerPixel * optics.signs.y,
  }
}

/**
 * Double-click to move: centres the live image on the point, or goes as far towards it as the
 * travel allows. While a move runs, further double-clicks are ignored.
 */
export function useClickToMove() {
  const camera = useCameraStore()
  const stage = useStageStore()
  const peripherals = usePeripheralsStore()
  const preferences = useNavigationPreferences()
  let moving = false

  return async (point: Point) => {
    if (!camera.region || !camera.sensor || moving) return
    moving = true
    try {
      const { dx, dy } = clickMove(point, {
        region: camera.region,
        pixelSize: camera.sensor.pixel_size,
        magnification: peripherals.objectives.current?.magnification ?? NOMINAL_MAGNIFICATION,
        signs: preferences.signs(),
      })
      // As far as the travel allows, towards the point.
      const { x, y } = stageWithinTravel({ x: dx, y: dy, z: 0 }, stage.position, stage.limits)
      if (x || y) await stage.moveBy({ dx: x, dy: y })
    } finally {
      moving = false
    }
  }
}
