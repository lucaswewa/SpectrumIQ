// Gating (ADR-0051): what the calibrations in force allow. A scan needs the objective's
// stage–camera calibration, to place its tiles; without a flat for a channel, or with a
// calibration months old, it may go on, with a warning.
import type { CalibrationStatus } from '@/stores/calibration'

/** A stage–camera calibration older than this is stale: worth doing again, not required. */
export const STALE_DAYS = 90

/** Whether a scan may run, what stops it, and what it should know. */
export interface Gate {
  ready: boolean
  blockers: string[]
  warnings: string[]
}

const DAY_MS = 24 * 3600 * 1000

/**
 * The gate of a scan with objective `objective` in channels `channels` (`null` for frames taken
 * with none), by `status`, at `now`.
 */
export function scanGate(
  status: CalibrationStatus | undefined,
  objective: number,
  channels: (number | null)[],
  now = Date.now(),
): Gate {
  const found = status?.find((entry) => entry.objective === objective)
  const [blockers, warnings]: [string[], string[]] = [[], []]
  if (!status) blockers.push("The calibrations aren't known yet.")
  else if (!found) blockers.push(`Objective ${objective} isn't configured.`)
  else if (!found.calibration) {
    blockers.push(`${found.name} has no stage–camera calibration: tiles can't be placed.`)
  } else {
    const days = Math.floor((now - Date.parse(found.calibration.created)) / DAY_MS)
    if (days > STALE_DAYS) {
      warnings.push(
        `${found.name} was calibrated ${days} days ago: calibrate again if the camera, its adapter or the objective was moved.`,
      )
    }
  }
  for (const channel of channels) {
    const shading = found?.shading.find((entry) => (entry.channel ?? null) === channel)
    if (found && !shading?.flat) {
      const name = shading?.name ?? `channel ${channel}`
      warnings.push(`${name} has no flat with ${found.name}: the mosaic may show its seams.`)
    }
  }
  return { ready: blockers.length === 0, blockers, warnings }
}
