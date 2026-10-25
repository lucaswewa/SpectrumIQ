/** Distances, velocities, or a direction (-1, 0 or 1), on each axis. */
export interface Axes {
  x: number
  y: number
  z: number
}

export const STILL: Axes = { x: 0, y: 0, z: 0 }

/**
 * Sends moves one at a time, as the stage takes them: what is asked while one runs is added up,
 * and sent as one move when it ends. So quick taps, or a wheel's turns, are never refused.
 */
export class Stepper {
  private pending = STILL
  private running?: Promise<void>

  /** `move` moves by a distance, and resolves when the move ends, or fails. */
  constructor(private readonly move: (by: Axes) => Promise<unknown>) {}

  add(by: Axes) {
    this.pending = sum(this.pending, by)
    if (!this.running && !isStill(this.pending)) this.running = this.drain()
  }

  /** Forgets what is still to be sent: after STOP. */
  clear() {
    this.pending = STILL
  }

  /** Resolves once every move asked has ended. */
  async idle() {
    while (this.running) await this.running
  }

  private async drain() {
    while (!isStill(this.pending)) {
      const by = this.pending
      this.pending = STILL
      await this.move(by).catch(() => {})
    }
    this.running = undefined
  }
}

export interface JogOptions {
  /** Moves by a distance, resolving when the move ends. */
  step: (by: Axes) => Promise<unknown>
  /** Moves at a velocity until stopped, replacing any jog in progress. */
  jog: (velocity: Axes) => void
  /** Stops the stage. */
  stop: () => void
  /** How far one step goes on each axis. */
  steps: () => Axes
  /** How long a press is held before it moves continuously, in ms. */
  hold?: number
  /** How fast a held press moves, in steps a second. */
  rate?: number
}

/** How long a press is held before it moves continuously, in ms. */
export const HOLD_MS = 300
/** How fast a held press moves, in steps a second. */
export const JOG_RATE = 4

/**
 * Turns presses of the d-pad, the coarse-Z buttons and the keys into moves (ADR-0039).
 *
 * A press moves one step. If it is still held after `hold`, and once that step has ended, the
 * stage jogs at `rate` steps a second, until every press is released: then it stops. Presses
 * combine (↑ and → move diagonally), and a press of something already held, such as a key's
 * auto-repeat, is ignored.
 */
export class JogController {
  private readonly held = new Map<string, Axes>()
  private readonly stepper: Stepper
  private timer?: ReturnType<typeof setTimeout>
  private continuous = false
  /** Counts the presses' ends, so that a hold that ended is not resumed. */
  private ended = 0

  constructor(private readonly options: JogOptions) {
    this.stepper = new Stepper(options.step)
  }

  /** `source` (a key, or a button) is pressed, towards `direction`. */
  press(source: string, direction: Axes) {
    if (this.held.has(source)) return
    this.held.set(source, direction)
    if (this.continuous) return this.send()
    this.stepper.add(scale(direction, this.options.steps()))
    if (this.timer !== undefined) return
    const ended = this.ended
    this.timer = setTimeout(async () => {
      // The stage takes one move at a time: the step ends first.
      await this.stepper.idle()
      if (ended !== this.ended) return
      this.continuous = true
      this.send()
    }, this.options.hold ?? HOLD_MS)
  }

  /** `source` is released. */
  release(source: string) {
    if (!this.held.delete(source)) return
    if (this.held.size === 0) this.end()
    else if (this.continuous) this.send()
  }

  /** Everything is released: when the window loses focus, or the page is left. */
  releaseAll() {
    if (this.held.size === 0) return
    this.held.clear()
    this.end()
  }

  /** STOP: forgets every press and step still to come. The caller stops the stage. */
  cancel() {
    this.stepper.clear()
    this.held.clear()
    this.continuous = false
    this.end()
  }

  private end() {
    clearTimeout(this.timer)
    this.timer = undefined
    this.ended += 1
    if (this.continuous) this.options.stop()
    this.continuous = false
  }

  /** The held directions together, each axis kept to -1, 0 or 1. */
  private direction(): Axes {
    const total = [...this.held.values()].reduce(sum, STILL)
    return { x: Math.sign(total.x), y: Math.sign(total.y), z: Math.sign(total.z) }
  }

  /** Jogs towards the held directions: still, if they cancel out. */
  private send() {
    const rate = this.options.rate ?? JOG_RATE
    const [steps, towards] = [this.options.steps(), this.direction()]
    this.options.jog({
      x: towards.x * steps.x * rate,
      y: towards.y * steps.y * rate,
      z: towards.z * steps.z * rate,
    })
  }
}

function sum(a: Axes, b: Axes): Axes {
  return { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z }
}

function scale(direction: Axes, steps: Axes): Axes {
  return { x: direction.x * steps.x, y: direction.y * steps.y, z: direction.z * steps.z }
}

/**
 * A move of `by` from `at`, cut to stay within `travel`, and never turned the other way: so a
 * step at a limit goes no further, rather than being refused.
 */
export function withinTravel(by: number, at?: number, travel?: { min: number; max: number }) {
  if (at === undefined || !travel) return by
  return by > 0
    ? Math.max(0, Math.min(by, travel.max - at))
    : Math.min(0, Math.max(by, travel.min - at))
}

/** A stage move of `by` from `at`, cut on each axis to stay within `travel` (`withinTravel`). */
export function stageWithinTravel(by: Axes, at?: Axes, travel?: { min: Axes; max: Axes }): Axes {
  const cut = (axis: keyof Axes) =>
    withinTravel(by[axis], at?.[axis], travel && { min: travel.min[axis], max: travel.max[axis] })
  return { x: cut('x'), y: cut('y'), z: cut('z') }
}

export function isStill(a: Axes) {
  return a.x === 0 && a.y === 0 && a.z === 0
}
