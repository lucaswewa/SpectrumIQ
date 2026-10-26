// The gating rules (ADR-0051): a scan needs its objective's stage–camera calibration; a missing
// flat, or a calibration months old, only warns. REQ-CAL-001, REQ-CAL-002.
import { describe, expect, it } from 'vitest'

import { scanGate, STALE_DAYS } from '@/calibration/gating'
import type { CalibrationStatus } from '@/stores/calibration'

const NOW = Date.parse('2026-10-25T12:00:00Z')
const DAY = 24 * 3600 * 1000

function status(calibrated: string | null, flat: boolean): CalibrationStatus {
  return [
    {
      objective: 1,
      name: '10x/0.30',
      calibration: calibrated ? ({ created: calibrated } as never) : null,
      shading: [
        { channel: 1, name: 'Brightfield', dark: null, flat: flat ? ({} as never) : null },
        { channel: 2, name: 'DAPI', dark: null, flat: null },
      ],
    },
  ]
}

describe('the gating rules', () => {
  it('block a scan until its objective has a stage–camera calibration', () => {
    const gate = scanGate(status(null, true), 1, [1], NOW)
    expect(gate.ready).toBe(false)
    expect(gate.blockers).toEqual([
      "10x/0.30 has no stage–camera calibration: tiles can't be placed.",
    ])
    expect(scanGate(undefined, 1, [], NOW).blockers).toEqual(["The calibrations aren't known yet."])
    expect(scanGate(status(null, true), 3, [], NOW).blockers).toEqual([
      "Objective 3 isn't configured.",
    ])
  })

  it('let it run once calibrated, warning of each channel without a flat', () => {
    const gate = scanGate(status('2026-10-20T12:00:00Z', true), 1, [1, 2], NOW)
    expect(gate).toEqual({
      ready: true,
      blockers: [],
      warnings: ['DAPI has no flat with 10x/0.30: the mosaic may show its seams.'],
    })
    expect(scanGate(status('2026-10-20T12:00:00Z', true), 1, [1], NOW).warnings).toEqual([])
  })

  it('warn of a calibration months old', () => {
    const old = new Date(NOW - (STALE_DAYS + 10) * DAY).toISOString()
    const gate = scanGate(status(old, true), 1, [1], NOW)
    expect(gate.ready).toBe(true)
    expect(gate.warnings[0]).toContain(`calibrated ${STALE_DAYS + 10} days ago`)
  })
})
