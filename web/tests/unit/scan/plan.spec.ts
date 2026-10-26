// REQ-WF-001, REQ-WF-002, REQ-WF-003: the scan form, checked as the engine would, and its tile
// scan valid against the `workflow` Thing's schema in the OpenAPI document (ADR-0057).
import { describe, expect, it } from 'vitest'

import document from '@/api/generated/openapi.json'
import { duration, newForm, problems, request, size, type ScanForm } from '@/scan/plan'

interface Schema {
  type?: string
  properties?: Record<string, Schema>
  required?: string[]
  items?: Schema
  oneOf?: Schema[]
  const?: unknown
  minimum?: number
  maximum?: number
  minItems?: number
  maxItems?: number
}

const SCHEMA = document.components.schemas.workflow_tile_scan_input as unknown as Schema

/** Whether `value` fits `schema`, as far as the API's schemas go: TD's terms, unions as `oneOf`. */
function fits(schema: Schema, value: unknown): boolean {
  if (schema.oneOf) return schema.oneOf.filter((branch) => fits(branch, value)).length === 1
  if ('const' in schema && value !== schema.const) return false
  switch (schema.type) {
    case 'null':
      return value === null
    case 'string':
      return typeof value === 'string'
    case 'boolean':
      return typeof value === 'boolean'
    case 'integer':
    case 'number': {
      const number =
        typeof value === 'number' && (schema.type === 'number' || Number.isInteger(value))
      return (
        number && !(value < (schema.minimum ?? -Infinity) || value > (schema.maximum ?? Infinity))
      )
    }
    case 'array':
      return (
        Array.isArray(value) &&
        value.length >= (schema.minItems ?? 0) &&
        value.length <= (schema.maxItems ?? Infinity) &&
        value.every((item) => !schema.items || fits(schema.items, item))
      )
    case 'object': {
      if (typeof value !== 'object' || value === null || Array.isArray(value)) return false
      const object = value as Record<string, unknown>
      const known = schema.properties ?? {}
      return (
        (schema.required ?? []).every((key) => key in object) &&
        Object.entries(object).every(([key, item]) => key in known && fits(known[key]!, item))
      )
    }
  }
  return true
}

const drawn = (changes: Partial<ScanForm> = {}): ScanForm => ({
  ...newForm(),
  region: { shape: 'rect', x0: -21_000, y0: -16_000, x1: -19_000, y1: -14_000 },
  ...changes,
})

describe('the scan form', () => {
  it('asks for a tile scan the server takes, whatever the focusing', () => {
    const forms = [
      drawn(),
      drawn({ channels: [1, 2], order: 'nearest', overlap: 0.25 }),
      drawn({ focus: 'fixed', focusZ: 2010 }),
      drawn({ focus: 'autofocus', rangeUm: 30 }),
      drawn({
        focus: 'map',
        places: [
          [-21_000, -16_000],
          [-19_000, -16_000],
          [-20_000, -14_000],
        ],
      }),
      drawn({
        region: {
          shape: 'polygon',
          points: [
            [0, 0],
            [-500, 0],
            [-250, -400],
          ],
        },
        stack: { on: true, rangeUm: 10, stepUm: 2, direction: 'down' },
      }),
    ]
    for (const form of forms) {
      expect(problems(form)).toEqual([])
      const asked = request(form)!
      expect(fits(SCHEMA, asked), JSON.stringify(asked)).toBe(true)
    }
    expect(request(drawn({ focus: 'map' }))!.focus).toEqual({ mode: 'surface' })
    expect(request(newForm())).toBeUndefined()
    // Not what the schema takes.
    expect(fits(SCHEMA, { ...request(drawn()), order: 'spiral' })).toBe(false)
    expect(fits(SCHEMA, { ...request(drawn()), name: undefined })).toBe(false)
  })

  it('says what is wrong before the server is asked', () => {
    expect(problems(newForm())).toEqual(['Draw the region to scan on the map.'])
    expect(problems(drawn({ name: ' ', overlap: 0.95 }))).toEqual([
      'The scan needs a name.',
      'The overlap is from 0 to 90 %.',
    ])
    expect(problems(drawn({ focus: 'map', places: [[0, 0]] }))).toEqual([
      'A focus map needs three places: 1 picked.',
    ])
    const stack = { on: true, rangeUm: 10, stepUm: 0, direction: 'up' as const }
    expect(problems(drawn({ stack }))).toHaveLength(1)
    expect(problems(drawn({ stack: { ...stack, rangeUm: 4000, stepUm: 2 } }))).toHaveLength(1)
  })

  it('reads its estimate as a person would', () => {
    expect([duration(42.4), duration(754), duration(12_000)]).toEqual([
      '42 s',
      '12 min 34 s',
      '3 h 20 min',
    ])
    expect([size(512), size(3_400), size(3.2e9), size(512e9)]).toEqual([
      '512 B',
      '3.4 kB',
      '3.2 GB',
      '512 GB',
    ])
  })
})
