// @vitest-environment node
import { describe, expect, it } from 'vitest'

import { multipartBoundary, parseMultipart } from '@/api/wot/mjpeg'

import { jpeg, mjpegPart } from './fakeServer'

const bytes = (...parts: (string | Uint8Array)[]) => {
  const encoded = parts.map((part) =>
    typeof part === 'string' ? new TextEncoder().encode(part) : part,
  )
  const all = new Uint8Array(encoded.reduce((length, part) => length + part.length, 0))
  encoded.reduce((offset, part) => (all.set(part, offset), offset + part.length), 0)
  return all
}

async function parse(...chunks: Uint8Array[]): Promise<number[][]> {
  async function* source() {
    yield* chunks
  }
  const parts: number[][] = []
  for await (const part of parseMultipart(source(), 'frame')) parts.push([...part])
  return parts
}

describe('parseMultipart', () => {
  it('reads each part, skipping anything before the first boundary', async () => {
    const stream = bytes('preamble\r\n', mjpegPart(jpeg(1)), mjpegPart(jpeg(2)))
    expect(await parse(stream)).toEqual([[...jpeg(1)], [...jpeg(2)]])
  })

  it('reads parts split anywhere between chunks, even within a boundary', async () => {
    const stream = bytes(
      mjpegPart(jpeg(1)),
      '--frame\r\nContent-Type: text/plain\r\n\r\nnot a JPEG\r\n',
      mjpegPart(jpeg(2)),
      '--frame--\r\n',
    )
    const expected = [[...jpeg(1)], [...new TextEncoder().encode('not a JPEG')], [...jpeg(2)]]
    expect(await parse(stream)).toEqual(expected)
    for (let i = 1; i < stream.length; i++) {
      expect(await parse(stream.slice(0, i), stream.slice(i))).toEqual(expected)
    }
    expect(await parse(...[...stream].map((byte) => Uint8Array.of(byte)))).toEqual(expected)
  })

  it('honours Content-Length, even when the body looks like a boundary', async () => {
    const body = 'a\r\n--frame\r\nb'
    const stream = bytes(`--frame\r\nContent-Length: ${body.length}\r\n\r\n${body}\r\n--frame--`)
    expect(await parse(stream)).toEqual([[...new TextEncoder().encode(body)]])
  })

  it('reads a part with no headers, and stops at the closing boundary', async () => {
    const stream = bytes('--frame\r\n\r\nbare\r\n--frame--\r\n', mjpegPart(jpeg(3)))
    expect(await parse(stream)).toEqual([[...new TextEncoder().encode('bare')]])
  })

  it('hands on a JPEG as soon as it ends, without waiting for the next boundary', async () => {
    let more!: (chunk: Uint8Array) => void
    async function* source() {
      yield bytes(mjpegPart(jpeg(1)), '--frame\r\nContent-Type: image/jpeg\r\n\r\n')
      yield jpeg(2).slice(0, 5)
      yield await new Promise<Uint8Array>((resolve) => (more = resolve))
    }
    const parts = parseMultipart(source(), 'frame')
    expect([...(await parts.next()).value!]).toEqual([...jpeg(1)])
    // The second JPEG is incomplete: nothing until the rest arrives.
    const second = parts.next()
    const pending = Symbol('pending')
    const early = await Promise.race([second, new Promise((r) => setTimeout(() => r(pending), 10))])
    expect(early).toBe(pending)
    more(jpeg(2).slice(5))
    expect([...(await second).value!]).toEqual([...jpeg(2)])
  })
})

describe('multipartBoundary', () => {
  it('reads the boundary of a multipart/x-mixed-replace type', () => {
    expect(multipartBoundary('multipart/x-mixed-replace; boundary=frame')).toBe('frame')
    expect(multipartBoundary('Multipart/X-Mixed-Replace;charset=x; BOUNDARY="a=b"')).toBe('a=b')
    expect(multipartBoundary('multipart/x-mixed-replace')).toBeUndefined()
    expect(multipartBoundary('image/jpeg; boundary=frame')).toBeUndefined()
    expect(multipartBoundary(null)).toBeUndefined()
  })
})
