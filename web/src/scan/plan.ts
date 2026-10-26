// The scan setup's form (ADR-0057): what is asked, the tile scan it asks the server for, what is
// wrong with it before the server is asked, and how an estimate reads.
import type { TileScanRequest } from '@/stores/workflow'

import type { Point } from './geometry'

/** A region of the stage, as a tile scan takes it. */
export type Region = TileScanRequest['region']

/** How the tiles are focused: as the focus is, at one focus, on a focus map, or each autofocused. */
export type FocusMode = 'none' | 'fixed' | 'map' | 'autofocus'

/** What the form asks for. */
export interface ScanForm {
  name: string
  region?: Region
  channels: number[]
  /** Of the frame, from 0 to 0.9. */
  overlap: number
  order: 'snake' | 'raster' | 'nearest'
  focus: FocusMode
  /** For `fixed`, in µm. */
  focusZ: number
  /** For `map`: the places, in stage µm. */
  places: Point[]
  /** For `map` and `autofocus`: each autofocus's range, in µm. */
  rangeUm: number
  stack: { on: boolean; rangeUm: number; stepUm: number; direction: 'up' | 'down' }
}

/** A new form: no region yet, the channel in use, 10 % overlap, a snake, as the focus is. */
export function newForm(): ScanForm {
  return {
    name: `scan-${new Date().toISOString().slice(0, 16).replace(/[-:T]/g, '')}`,
    channels: [],
    overlap: 0.1,
    order: 'snake',
    focus: 'none',
    focusZ: 0,
    places: [],
    rangeUm: 50,
    stack: { on: false, rangeUm: 10, stepUm: 2, direction: 'up' },
  }
}

/** The most planes a stack takes, as the engine allows. */
const MAX_PLANES = 1000

/** What is wrong with `form`, as the engine would say, before asking the server. */
export function problems(form: ScanForm): string[] {
  const found: string[] = []
  const finite = (value: number) => Number.isFinite(value)
  if (!form.name.trim()) found.push('The scan needs a name.')
  if (!form.region) found.push('Draw the region to scan on the map.')
  if (!(form.overlap >= 0 && form.overlap <= 0.9)) found.push('The overlap is from 0 to 90 %.')
  if (form.focus === 'fixed' && !finite(form.focusZ)) found.push('The focus needs a value.')
  if (form.focus === 'map' && form.places.length < 3) {
    found.push(`A focus map needs three places: ${form.places.length} picked.`)
  }
  if ((form.focus === 'map' || form.focus === 'autofocus') && !(form.rangeUm > 0)) {
    found.push("The autofocus's range must be more than 0.")
  }
  const { on, rangeUm, stepUm } = form.stack
  if (on && !(stepUm > 0 && rangeUm >= 0 && rangeUm / stepUm + 1 <= MAX_PLANES)) {
    found.push(`A z-stack takes from 1 to ${MAX_PLANES} planes, a step of more than 0 apart.`)
  }
  return found
}

/** The tile scan `form` asks for: none until it has a region. */
export function request(form: ScanForm): TileScanRequest | undefined {
  if (!form.region) return undefined
  const focus: TileScanRequest['focus'] = {
    none: { mode: 'none' as const },
    fixed: { mode: 'fixed' as const, z: form.focusZ },
    map: { mode: 'surface' as const },
    autofocus: { mode: 'autofocus' as const, range_um: form.rangeUm },
  }[form.focus]
  const { on, rangeUm, stepUm, direction } = form.stack
  return {
    name: form.name.trim(),
    region: form.region,
    overlap: form.overlap,
    order: form.order,
    channels: form.channels,
    focus,
    z_stack: on ? { range_um: rangeUm, step_um: stepUm, direction } : null,
    focus_map:
      form.focus === 'map'
        ? { places: form.places, surface: 'thin_plate', range_um: form.rangeUm }
        : null,
  }
}

/** `seconds` as a person reads it: `45 s`, `12 min 5 s`, `3 h 20 min`. */
export function duration(seconds: number): string {
  const s = Math.round(seconds)
  if (s < 60) return `${s} s`
  if (s < 3600) return `${Math.floor(s / 60)} min ${s % 60} s`
  return `${Math.floor(s / 3600)} h ${Math.round((s % 3600) / 60)} min`
}

/** `bytes` in the largest unit that keeps them over 1: `512 kB`, `3.4 GB`. */
export function size(bytes: number): string {
  const units = ['B', 'kB', 'MB', 'GB', 'TB']
  let [value, unit] = [bytes, 0]
  while (value >= 1000 && unit < units.length - 1) [value, unit] = [value / 1000, unit + 1]
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`
}
