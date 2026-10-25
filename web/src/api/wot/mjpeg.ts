/**
 * Reading `multipart/x-mixed-replace` streams, as MJPEG cameras serve them:
 * each part is one frame, usually a JPEG.
 */

const encoder = new TextEncoder()
const decoder = new TextDecoder()
const CRLF = encoder.encode('\r\n')
const HEADERS_END = encoder.encode('\r\n\r\n')

/** The boundary a `multipart/x-mixed-replace` content type names, if it is one. */
export function multipartBoundary(contentType: string | null): string | undefined {
  const [type, ...parameters] = (contentType ?? '').split(';').map((part) => part.trim())
  if (type?.toLowerCase() !== 'multipart/x-mixed-replace') return undefined
  for (const parameter of parameters) {
    const [name, value] = parameter.split(/=(.*)/, 2)
    if (name?.trim().toLowerCase() === 'boundary' && value) {
      return value.trim().replace(/^"(.*)"$/, '$1')
    }
  }
  return undefined
}

/**
 * The parts of a multipart stream, from its bytes in chunks that may split
 * it anywhere. Anything before the first boundary is skipped.
 *
 * A part ends after its `Content-Length`, if it has one, or else at the next
 * boundary. A JPEG part with no length is handed on as soon as its bytes end
 * with the JPEG end-of-image marker: waiting for the next boundary would hold
 * every frame back until the next one began.
 */
export async function* parseMultipart(
  chunks: AsyncIterable<Uint8Array>,
  boundary: string,
): AsyncGenerator<Uint8Array<ArrayBuffer>> {
  const delimiter = encoder.encode(`--${boundary}`)
  const nextPart = encoder.encode(`\r\n--${boundary}`)
  const buffer = new ByteQueue()
  let state: 'boundary' | 'headers' | 'body' = 'boundary'
  let length: number | undefined
  let jpeg = false
  /** How much of the body has been searched for the next boundary. */
  let searched = 0

  for await (const chunk of chunks) {
    buffer.push(chunk)
    for (;;) {
      if (state === 'boundary') {
        const at = buffer.indexOf(delimiter)
        const lineEnd = at < 0 ? -1 : buffer.indexOf(CRLF, at + delimiter.length)
        if (lineEnd < 0) {
          // Keep what may be the start of a delimiter.
          if (at < 0) buffer.drop(Math.max(0, buffer.length - delimiter.length))
          break
        }
        // `--boundary--` closes the stream.
        const after = at + delimiter.length
        if (buffer.at(after) === 0x2d && buffer.at(after + 1) === 0x2d) return
        buffer.drop(lineEnd + CRLF.length)
        state = 'headers'
      }
      if (state === 'headers') {
        let headers = ''
        if (buffer.startsWith(CRLF)) {
          // No headers: the blank line comes first.
          buffer.drop(CRLF.length)
        } else {
          const end = buffer.indexOf(HEADERS_END)
          if (end < 0) break
          headers = decoder.decode(buffer.take(end))
          buffer.drop(HEADERS_END.length)
        }
        const contentLength = /^content-length:\s*(\d+)\s*$/im.exec(headers)?.[1]
        length = contentLength === undefined ? undefined : Number(contentLength)
        jpeg = /^content-type:\s*image\/jpeg\s*(;|$)/im.test(headers)
        state = 'body'
        searched = 0
      }
      if (length !== undefined) {
        if (buffer.length < length) break
        yield buffer.take(length)
        state = 'boundary'
        continue
      }
      const at = buffer.indexOf(nextPart, searched)
      if (at >= 0) {
        yield buffer.take(at)
        state = 'boundary'
        continue
      }
      const complete = jpeg ? jpegEnd(buffer) : -1
      if (complete >= 0) {
        yield buffer.take(complete)
        state = 'boundary'
        continue
      }
      searched = Math.max(0, buffer.length - nextPart.length + 1)
      break
    }
  }
}

/**
 * Where a JPEG at the start of `buffer` ends, if the buffer ends with it:
 * with the end-of-image marker, and perhaps the CRLF before a boundary.
 */
function jpegEnd(buffer: ByteQueue): number {
  if (buffer.at(0) !== 0xff || buffer.at(1) !== 0xd8) return -1
  const end = buffer.endsWith(CRLF) ? buffer.length - CRLF.length : buffer.length
  return end >= 4 && buffer.at(end - 2) === 0xff && buffer.at(end - 1) === 0xd9 ? end : -1
}

/** Bytes received and not yet used: appended at the end, taken from the start. */
class ByteQueue {
  private bytes: Uint8Array<ArrayBuffer> = new Uint8Array(0)
  private start = 0

  get length() {
    return this.bytes.length - this.start
  }

  push(chunk: Uint8Array) {
    const next = new Uint8Array(this.length + chunk.length)
    next.set(this.bytes.subarray(this.start))
    next.set(chunk, this.length)
    this.bytes = next
    this.start = 0
  }

  at(index: number): number | undefined {
    return index < this.length ? this.bytes[this.start + index] : undefined
  }

  /** Where `pattern` first occurs, at or after `from`, or -1. */
  indexOf(pattern: Uint8Array, from = 0): number {
    const last = this.bytes.length - pattern.length
    for (let i = this.bytes.indexOf(pattern[0]!, this.start + from); i >= 0 && i <= last;) {
      let j = 1
      while (j < pattern.length && this.bytes[i + j] === pattern[j]) j++
      if (j === pattern.length) return i - this.start
      i = this.bytes.indexOf(pattern[0]!, i + 1)
    }
    return -1
  }

  startsWith(pattern: Uint8Array) {
    return pattern.every((byte, i) => this.at(i) === byte)
  }

  endsWith(pattern: Uint8Array) {
    const offset = this.length - pattern.length
    return offset >= 0 && pattern.every((byte, i) => this.at(offset + i) === byte)
  }

  /** Removes the first `count` bytes, and returns a copy of them. */
  take(count: number): Uint8Array<ArrayBuffer> {
    const taken = this.bytes.slice(this.start, this.start + count)
    this.start += count
    return taken
  }

  drop(count: number) {
    this.start += count
  }
}

/** A response body's chunks of bytes. */
export async function* byteChunks(body: ReadableStream<Uint8Array>): AsyncGenerator<Uint8Array> {
  const reader = body.getReader()
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      yield value
    }
  } finally {
    reader.releaseLock()
  }
}
