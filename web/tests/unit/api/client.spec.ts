// @vitest-environment node
import { describe, expect, it } from 'vitest'

import { WotClient } from '@/api/wot/client'
import { ApiError } from '@/api/wot/errors'
import { formUrl } from '@/api/wot/td'

import { fakeServer, json, stage } from './fakeServer'

const BASE = 'http://microscope.test/api/v1/'

function connect(routes: Parameters<typeof fakeServer>[0], headers?: () => Record<string, string>) {
  const server = fakeServer(routes)
  const client = new WotClient({ baseUrl: BASE, fetch: server.fetch, headers })
  return { client, thing: client.consume(stage), received: server.received }
}

async function failure(promise: Promise<unknown>): Promise<ApiError> {
  const error = await promise.then(
    () => undefined,
    (reason: unknown) => reason,
  )
  expect(error).toBeInstanceOf(ApiError)
  return error as ApiError
}

describe('formUrl', () => {
  it('finds the form for an operation, resolved against the base', () => {
    const forms = stage.properties!.position!.forms
    expect(formUrl(forms, 'readproperty', 'http://lab-pc:5000/')?.href).toBe(
      'http://lab-pc:5000/api/v1/stage/position',
    )
    expect(formUrl(forms, 'observeproperty', BASE, { sse: true })?.href).toBe(
      'http://microscope.test/api/v1/stage/position',
    )
    // Not the WebSocket form, which also observes.
    expect(formUrl(forms, 'observeproperty', BASE)).toBeUndefined()
    expect(formUrl(forms, 'writeproperty', BASE)).toBeUndefined()
  })
})

describe('WotClient', () => {
  it('lists the Things’ descriptions', async () => {
    const { client, received } = connect({ 'GET /api/v1/thing_descriptions/': { stage } })
    expect(await client.thingDescriptions()).toEqual({ stage })
    expect(received[0]!.headers.get('Accept')).toBe('application/json')
  })

  it('resolves forms against the client’s base when a description has none', async () => {
    const server = fakeServer({ 'GET /api/v1/stage/step': 5 })
    const client = new WotClient({ baseUrl: 'http://other.test:5000/api/v1/', fetch: server.fetch })
    await client.consume({ ...stage, base: undefined }).readProperty('step')
    expect(server.received[0]!.url.href).toBe('http://other.test:5000/api/v1/stage/step')
  })

  it('reads a property', async () => {
    const { thing } = connect({ 'GET /api/v1/stage/position': { x: 1, y: 2, z: 3 } })
    expect(await thing.readProperty('position')).toEqual({ x: 1, y: 2, z: 3 })
  })

  it('writes any JSON value, false and 0 included', async () => {
    const { thing, received } = connect({ 'PUT /api/v1/stage/step': null })
    for (const value of [false, 0, null, '', { a: 1 }]) await thing.writeProperty('step', value)
    expect(received.map((request) => request.body)).toEqual(['false', '0', 'null', '""', '{"a":1}'])
    expect(received[0]!.headers.get('Content-Type')).toBe('application/json')
  })

  it('resets a property', async () => {
    const { thing, received } = connect({ 'POST /api/v1/stage/step/reset': null })
    await thing.resetProperty('step')
    expect(received[0]!.body).toBeUndefined()
  })

  it('refuses an affordance the description lacks', async () => {
    const { thing } = connect({})
    await expect(thing.writeProperty('position', 1)).rejects.toThrow(
      'Stage has no writeproperty form for property "position".',
    )
    await expect(thing.readProperty('nothing')).rejects.toThrow('no readproperty form')
  })

  it('adds the header hook’s headers to every request', async () => {
    const { thing, received } = connect({ 'GET /api/v1/stage/step': 1 }, () => ({
      Authorization: 'Bearer t',
    }))
    await thing.readProperty('step')
    expect(received[0]!.headers.get('Authorization')).toBe('Bearer t')
  })
})

describe('ApiError', () => {
  it('is `invalid` for a 422, with the validation list', async () => {
    const body = { detail: [{ type: 'int_parsing', loc: ['body'], msg: 'Not an integer' }] }
    const { thing } = connect({ 'PUT /api/v1/stage/step': () => json(body, 422) })
    const error = await failure(thing.writeProperty('step', 'x'))
    expect([error.kind, error.status, error.message]).toEqual(['invalid', 422, '1 invalid input'])
    expect(error.body).toEqual(body)
  })

  it('is `lock-busy` when the global lock is taken', async () => {
    const body = { title: 'GlobalLockBusyError', detail: 'Busy.', status: 409, type: null }
    const { thing } = connect({ 'PUT /api/v1/stage/step': () => json(body, 409) })
    const error = await failure(thing.writeProperty('step', 1))
    expect([error.kind, error.status, error.message]).toEqual(['lock-busy', 409, 'Busy.'])
  })

  it('is `http` for other errors, with their message', async () => {
    const { thing } = connect({
      'GET /api/v1/stage/step': () => new Response('Oops', { status: 500 }),
    })
    expect(await failure(thing.readProperty('position'))).toMatchObject({
      kind: 'http',
      status: 404,
      message: 'Not Found',
    })
    expect(await failure(thing.readProperty('step'))).toMatchObject({
      status: 500,
      message: 'Oops',
    })
  })

  it('is `network` when there is no answer, and `cancelled` when aborted', async () => {
    const offline = new WotClient({
      baseUrl: BASE,
      fetch: () => Promise.reject(new TypeError('Failed to fetch')),
    })
    expect(await failure(offline.thingDescriptions())).toMatchObject({ kind: 'network' })
    const { client } = connect({ 'GET /api/v1/thing_descriptions/': {} })
    const signal = AbortSignal.abort()
    expect(await failure(client.thingDescriptions({ signal }))).toMatchObject({ kind: 'cancelled' })
  })
})
