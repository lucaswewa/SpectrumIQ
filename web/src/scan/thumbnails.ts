// A run's thumbnails (ADR-0057): each frame the run takes, as the overview stream next shows it,
// the latest first.
import { shallowRef } from 'vue'

/** A frame's indices. */
export interface Key {
  t: number
  c: number
  z: number
  p: number
}

/** A frame's thumbnail. */
export interface Thumbnail<I> {
  key: Key
  image: I
}

/** The latest `max` thumbnails; `drop` lets go of each one left out. */
export function useThumbnails<I>(max = 8, drop: (image: I) => void = () => {}) {
  const items = shallowRef<Thumbnail<I>[]>([])
  let waiting: Key | undefined

  return {
    items,
    /** A frame was taken: the stream's next image is its. */
    acquired(key: Key) {
      waiting = key
    },
    /** The frame the stream's image is of, once: none if no frame waits for it. */
    claim(): Key | undefined {
      const key = waiting
      waiting = undefined
      return key
    },
    /** Keeps `image` as `key`'s thumbnail, before the others. */
    add(key: Key, image: I) {
      const next = [{ key, image }, ...items.value]
      next.slice(max).forEach((old) => drop(old.image))
      items.value = next.slice(0, max)
    },
    clear() {
      items.value.forEach((old) => drop(old.image))
      items.value = []
      waiting = undefined
    },
  }
}
