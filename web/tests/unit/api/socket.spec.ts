// REQ-NET-002: the observation socket of a Thing (ADR-0037), against a fake socket.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { WotClient } from '@/api/wot/client'
import type { ApiError } from '@/api/wot/errors'
import type { ThingDescription } from '@/api/wot/td'
import { ThingSocket } from '@/api/wot/ws'

import { fakeServer, stage } from './fakeServer'
import { fakeSockets } from './fakeSocket'

const URL = 'ws://spectrumiq.test/api/v1/stage/ws'

describe('a Thing’s observation socket', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('opens with the first subscriber, and subscribes to each name once it is open', () => {
    const sockets = fakeSockets()
    const socket = new ThingSocket(URL, sockets.open)
    socket.subscribe('property', 'position', { onValue: () => {} })
    socket.subscribe('property', 'state', { onValue: () => {} })
    socket.subscribe('event', 'arrived', { onValue: () => {} })
    expect(sockets.opened).toHaveLength(1)
    expect(sockets.opened[0]!.sent).toEqual([])
    sockets.opened[0]!.open()
    expect(sockets.opened[0]!.sent).toEqual([
      { messageType: 'addPropertyObservation', data: { position: true, state: true } },
      { messageType: 'addEventSubscription', data: { arrived: true } },
    ])
  })

  it('subscribes a new name at once while open, and a known one not again', () => {
    const sockets = fakeSockets()
    const socket = new ThingSocket(URL, sockets.open)
    socket.subscribe('property', 'position', { onValue: () => {} })
    sockets.opened[0]!.open()
    socket.subscribe('property', 'position', { onValue: () => {} })
    socket.subscribe('property', 'homed', { onValue: () => {} })
    expect(sockets.opened[0]!.sent).toEqual([
      { messageType: 'addPropertyObservation', data: { position: true } },
      { messageType: 'addPropertyObservation', data: { homed: true } },
    ])
  })

  it('gives each subscriber its values, and an event’s data without its time', () => {
    const sockets = fakeSockets()
    const socket = new ThingSocket(URL, sockets.open)
    const [first, second, arrivals] = [vi.fn(), vi.fn(), vi.fn()]
    socket.subscribe('property', 'position', { onValue: first })
    socket.subscribe('property', 'position', { onValue: second })
    socket.subscribe('event', 'arrived', { onValue: arrivals })
    const wire = sockets.opened[0]!
    wire.open()
    wire.receive({ messageType: 'propertyStatus', data: { position: { x: -200, y: 0, z: 0 } } })
    wire.receive({ messageType: 'propertyStatus', data: { speed: 50 } })
    wire.receive({
      messageType: 'event',
      data: { arrived: { data: { x: -200, y: 0, z: 0 }, timestamp: '2026-10-24T22:00:00Z' } },
    })
    expect(first).toHaveBeenCalledExactlyOnceWith({ x: -200, y: 0, z: 0 })
    expect(second).toHaveBeenCalledExactlyOnceWith({ x: -200, y: 0, z: 0 })
    expect(arrivals).toHaveBeenCalledExactlyOnceWith({ x: -200, y: 0, z: 0 })
  })

  it('tells the subscribers of a name the server refuses', () => {
    const sockets = fakeSockets()
    const socket = new ThingSocket(URL, sockets.open)
    const errors: ApiError[] = []
    socket.subscribe('property', 'limits', { onValue: () => {}, onError: (e) => errors.push(e) })
    sockets.opened[0]!.open()
    sockets.opened[0]!.receive({
      messageType: 'response',
      operation: 'observeproperty',
      name: 'limits',
      error: {
        status: '403',
        title: 'Not Observable',
        detail: "Property 'limits' is not observable.",
      },
    })
    expect(errors.map((e) => [e.status, e.message])).toEqual([
      [403, "Property 'limits' is not observable."],
    ])
  })

  it('reconnects after 1 s, then 2, 4, …, and subscribes again to every name', async () => {
    const sockets = fakeSockets()
    const socket = new ThingSocket(URL, sockets.open)
    const errors: ApiError[] = []
    socket.subscribe('property', 'position', { onValue: () => {}, onError: (e) => errors.push(e) })
    socket.subscribe('event', 'arrived', { onValue: () => {} })
    sockets.opened[0]!.open()
    sockets.opened[0]!.drop()
    expect(errors.map((error) => error.kind)).toEqual(['network'])
    expect(socket.connected).toBe(false)

    await vi.advanceTimersByTimeAsync(999)
    expect(sockets.opened).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(1)
    expect(sockets.opened).toHaveLength(2)
    // It fails again before opening: the next try waits twice as long.
    sockets.opened[1]!.drop()
    await vi.advanceTimersByTimeAsync(1999)
    expect(sockets.opened).toHaveLength(2)
    await vi.advanceTimersByTimeAsync(1)
    expect(sockets.opened).toHaveLength(3)

    sockets.opened[2]!.open()
    expect(socket.connected).toBe(true)
    expect(sockets.opened[2]!.sent).toEqual([
      { messageType: 'addPropertyObservation', data: { position: true } },
      { messageType: 'addEventSubscription', data: { arrived: true } },
    ])
  })

  it('closes when its last subscriber stops, and stays closed', async () => {
    const sockets = fakeSockets()
    const socket = new ThingSocket(URL, sockets.open)
    const stopPosition = socket.subscribe('property', 'position', { onValue: () => {} })
    const stopArrived = socket.subscribe('event', 'arrived', { onValue: () => {} })
    sockets.opened[0]!.open()
    stopPosition()
    expect(sockets.opened[0]!.closed).toBe(false)
    stopArrived()
    expect(sockets.opened[0]!.closed).toBe(true)
    await vi.advanceTimersByTimeAsync(60_000)
    expect(sockets.opened).toHaveLength(1)
  })
})

describe('the client’s observations', () => {
  const describes = () => fakeServer({ 'GET /api/v1/stage/position': 0 })

  it('go over one socket per Thing, which its observations share', () => {
    const sockets = fakeSockets()
    const client = new WotClient({
      baseUrl: 'http://microscope.test/api/v1/',
      fetch: describes().fetch,
      webSocket: sockets.open,
    })
    const thing = client.consume(stage)
    const [positions, arrivals] = [vi.fn(), vi.fn()]
    thing.observeProperty('position', positions)
    thing.subscribeEvent('arrived', arrivals)
    expect(sockets.opened.map((socket) => socket.url)).toEqual([
      'ws://microscope.test/api/v1/stage/ws',
    ])
    sockets.opened[0]!.open()
    sockets.opened[0]!.receive({ messageType: 'propertyStatus', data: { position: 7 } })
    expect(positions).toHaveBeenCalledWith(7)
  })

  it('stop when their signal aborts', () => {
    const sockets = fakeSockets()
    const client = new WotClient({
      baseUrl: 'http://microscope.test/api/v1/',
      fetch: describes().fetch,
      webSocket: sockets.open,
    })
    const controller = new AbortController()
    client.consume(stage).observeProperty('position', () => {}, { signal: controller.signal })
    controller.abort()
    expect(sockets.opened[0]!.closed).toBe(true)
  })

  it('use server-sent events for a property without a socket form', async () => {
    const sockets = fakeSockets()
    const server = fakeServer({})
    const client = new WotClient({
      baseUrl: 'http://microscope.test/api/v1/',
      fetch: server.fetch,
      webSocket: sockets.open,
    })
    const sseOnly: ThingDescription = {
      ...stage,
      properties: {
        position: {
          ...stage.properties!.position!,
          forms: stage.properties!.position!.forms.filter((form) => !form.href.startsWith('ws')),
        },
      },
    }
    const stop = client.consume(sseOnly).observeProperty('position', () => {})
    await vi.waitFor(() => expect(server.received).toHaveLength(1))
    stop()
    expect(sockets.opened).toEqual([])
    expect(server.received[0]!.headers.get('Accept')).toBe('text/event-stream')
  })
})
