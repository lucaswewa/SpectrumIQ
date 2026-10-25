// @vitest-environment node
import { describe, expect, it } from 'vitest'

import { parseEventStream, textChunks, type SseMessage } from '@/api/wot/sse'

async function parse(...chunks: string[]): Promise<SseMessage[]> {
  async function* source() {
    yield* chunks
  }
  const messages: SseMessage[] = []
  for await (const message of parseEventStream(source())) messages.push(message)
  return messages
}

describe('parseEventStream', () => {
  it('reads events, their names, ids and multi-line data', async () => {
    expect(await parse('data: 1\n\nevent: x\nid: 7\ndata: a\ndata:b\n\n')).toEqual([
      { event: 'message', data: '1', id: '' },
      { event: 'x', data: 'a\nb', id: '7' },
    ])
  })

  it('reads events split anywhere between chunks', async () => {
    const stream = 'event: position\ndata: {"x": 1}\n\ndata: 2\n\n'
    const expected = await parse(stream)
    for (let i = 1; i < stream.length; i++) {
      expect(await parse(stream.slice(0, i), stream.slice(i))).toEqual(expected)
    }
    expect(await parse(...stream)).toEqual(expected)
  })

  it('accepts CRLF, CR and LF line endings, even a CRLF split between chunks', async () => {
    const expected = [
      { event: 'message', data: 'a', id: '' },
      { event: 'message', data: 'b', id: '' },
    ]
    expect(await parse('data: a\r\n\r\ndata: b\r\n\r\n')).toEqual(expected)
    expect(await parse('data: a\r\rdata: b\r\r')).toEqual(expected)
    expect(await parse('data: a\r', '\n\r', '\ndata: b\n\n')).toEqual(expected)
  })

  it('skips comments, events without data and a byte-order mark', async () => {
    expect(await parse('﻿: keep-alive\n\nevent: empty\n\ndata\n\n')).toEqual([
      { event: 'message', data: '', id: '' },
    ])
  })

  it('drops an event the stream ends in the middle of', async () => {
    expect(await parse('data: 1\n\ndata: 2\n')).toEqual([{ event: 'message', data: '1', id: '' }])
  })
})

describe('textChunks', () => {
  it('decodes UTF-8 split between chunks', async () => {
    const bytes = new TextEncoder().encode('data: µm\n\n')
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(bytes.slice(0, 7)) // splits the two bytes of µ
        controller.enqueue(bytes.slice(7))
        controller.close()
      },
    })
    const messages: SseMessage[] = []
    for await (const message of parseEventStream(textChunks(body))) messages.push(message)
    expect(messages.map((message) => message.data)).toEqual(['µm'])
  })
})
