import { describe, expect, it } from 'vitest'

import { containPoint } from '@/live/contain'

const image = { width: 800, height: 600 }

describe('containPoint', () => {
  it('places a point on an image letterboxed above and below', () => {
    // Drawn 1000 × 750, from y 125 to 875.
    const box = { width: 1000, height: 1000 }
    expect(containPoint({ x: 500, y: 500 }, box, image)).toEqual({ x: 0, y: 0 })
    expect(containPoint({ x: 750, y: 125 }, box, image)).toEqual({ x: 0.25, y: -0.5 })
    expect(containPoint({ x: 500, y: 100 }, box, image)).toBeUndefined()
  })

  it('places a point on an image letterboxed left and right', () => {
    // Drawn 400 × 300, from x 300 to 700.
    const box = { width: 1000, height: 300 }
    expect(containPoint({ x: 300, y: 225 }, box, image)).toEqual({ x: -0.5, y: 0.25 })
    expect(containPoint({ x: 710, y: 150 }, box, image)).toBeUndefined()
  })
})
