/** Wheel turns ("notches") in an event, whatever unit its delta is in. */
export function notches(event: Pick<WheelEvent, 'deltaY' | 'deltaMode'>): number {
  if (event.deltaMode === 1) return event.deltaY / 3 // lines: three to a notch
  if (event.deltaMode === 2) return event.deltaY // pages
  return event.deltaY / 100 // pixels: a hundred to a notch
}

/**
 * Focusing with the mouse wheel: turning it away from you focuses up, `step` µm a notch. `move`
 * takes the distance, and adds up what is turned while the focus moves (a `Stepper`).
 */
export function wheelFocus(move: (dz: number) => void, step: () => number) {
  return (event: WheelEvent) => {
    event.preventDefault()
    const dz = -notches(event) * step()
    if (dz !== 0) move(dz)
  }
}
