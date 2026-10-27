// A call made at most once an interval (ADR-0060): the first at once, then the latest of those
// made in between at the interval's end, so that the last call always goes through.

/** `call`, made at most once each `ms`; `cancel` drops a call waiting. */
export function throttle<A extends unknown[]>(call: (...args: A) => void, ms: number) {
  let last = -Infinity
  let waiting: A | undefined
  let timer: ReturnType<typeof setTimeout> | undefined
  const send = (args: A) => {
    last = Date.now()
    waiting = undefined
    call(...args)
  }
  const throttled = (...args: A) => {
    const wait = last + ms - Date.now()
    if (wait <= 0 && !timer) return send(args)
    waiting = args
    timer ??= setTimeout(
      () => {
        timer = undefined
        if (waiting) send(waiting)
      },
      Math.max(wait, 0),
    )
  }
  return Object.assign(throttled, {
    cancel() {
      clearTimeout(timer)
      timer = undefined
      waiting = undefined
    },
  })
}
