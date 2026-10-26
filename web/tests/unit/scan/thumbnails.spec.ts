// REQ-WF-005: a run's thumbnails, each frame taken paired with the stream's next image, the latest
// first (ADR-0057).
import { describe, expect, it } from 'vitest'

import { useThumbnails } from '@/scan/thumbnails'

const key = (p: number) => ({ t: 0, c: 1, z: 0, p })

describe("a run's thumbnails", () => {
  it('give each frame taken the next image, the latest first, keeping so many', () => {
    const dropped: string[] = []
    const thumbnails = useThumbnails<string>(3, (image) => dropped.push(image))
    // An image with no frame waiting isn't kept.
    expect(thumbnails.claim()).toBeUndefined()
    for (const p of [0, 1, 2, 3]) {
      thumbnails.acquired(key(p))
      const claimed = thumbnails.claim()!
      expect(claimed).toEqual(key(p))
      // The image after it is another frame's, if any.
      expect(thumbnails.claim()).toBeUndefined()
      thumbnails.add(claimed, `image ${p}`)
    }
    expect(thumbnails.items.value.map(({ key, image }) => [key.p, image])).toEqual([
      [3, 'image 3'],
      [2, 'image 2'],
      [1, 'image 1'],
    ])
    expect(dropped).toEqual(['image 0'])
    // A frame taken before the stream shows the last: the latest waits.
    thumbnails.acquired(key(4))
    thumbnails.acquired(key(5))
    expect(thumbnails.claim()).toEqual(key(5))
    thumbnails.clear()
    expect(thumbnails.items.value).toEqual([])
    expect(dropped).toEqual(['image 0', 'image 3', 'image 2', 'image 1'])
  })
})
