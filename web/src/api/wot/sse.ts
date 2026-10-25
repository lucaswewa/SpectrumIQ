/** One server-sent event. */
export interface SseMessage {
  /** Its name: `message` unless the server named it. */
  event: string
  data: string
  id: string
}

/**
 * Parses a `text/event-stream`, as the HTML standard describes, from its
 * text in chunks that may split lines, or even a CRLF, anywhere. Comments
 * (the server's keep-alives) and events without data are skipped.
 */
export async function* parseEventStream(chunks: AsyncIterable<string>): AsyncGenerator<SseMessage> {
  let buffer = ''
  let first = true
  let event = ''
  let data: string[] = []
  let id = ''

  /** Takes in one line, and returns the event it completes, if any. */
  function take(line: string): SseMessage | undefined {
    if (line === '') {
      const message =
        data.length > 0 ? { event: event || 'message', data: data.join('\n'), id } : undefined
      event = ''
      data = []
      return message
    }
    if (line.startsWith(':')) return undefined
    const colon = line.indexOf(':')
    const field = colon === -1 ? line : line.slice(0, colon)
    const value = colon === -1 ? '' : line.slice(colon + 1).replace(/^ /, '')
    if (field === 'data') data.push(value)
    else if (field === 'event') event = value
    else if (field === 'id' && !value.includes('\0')) id = value
    return undefined
  }

  for await (const chunk of chunks) {
    buffer += first ? chunk.replace(/^﻿/, '') : chunk
    first = false
    // A trailing CR may be half of a CRLF: keep it until the next chunk.
    const lines = buffer.split(/\r\n|\r(?!$)|\n/)
    buffer = lines.pop()!
    for (const line of lines) {
      const message = take(line)
      if (message) yield message
    }
  }
  // At the end, a kept CR did end a line.
  if (buffer.endsWith('\r')) {
    const message = take(buffer.slice(0, -1))
    if (message) yield message
  }
}

/** A response body as text chunks. */
export async function* textChunks(body: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const reader = body.getReader()
  const decoder = new TextDecoder()
  try {
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      yield decoder.decode(value, { stream: true })
    }
  } finally {
    reader.releaseLock()
  }
}
