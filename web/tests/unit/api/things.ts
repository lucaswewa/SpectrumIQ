import type { Form, ThingDescription } from '@/api/wot/td'

/** The server the test descriptions point at. */
export const BASE = 'http://spectrumiq.test/'

/** The forms teta-wot writes for a property, an action or an event of `thing`. */
function forms(
  thing: string,
  name: string,
  kind: 'property' | 'action' | 'event',
  observable = true,
) {
  const href = `/api/v1/${thing}/${name}`
  const socket: Form = { href: `ws://spectrumiq.test/api/v1/${thing}/ws`, op: [] }
  if (kind === 'action') return [{ href, op: ['invokeaction'] }]
  if (kind === 'event') return [{ ...socket, op: ['subscribeevent'] }]
  return observable
    ? [
        { href, op: ['readproperty', 'writeproperty'] },
        { ...socket, op: ['observeproperty'] },
      ]
    : [{ href, op: ['readproperty', 'writeproperty'] }]
}

/** A Thing's description, as teta-wot writes it: each property observable or not. */
export function describeThing(
  thing: string,
  properties: Record<string, boolean>,
  actions: string[] = [],
  events: string[] = [],
): ThingDescription {
  return {
    title: thing[0]!.toUpperCase() + thing.slice(1),
    base: BASE,
    properties: Object.fromEntries(
      Object.entries(properties).map(([name, observable]) => [
        name,
        { observable, forms: forms(thing, name, 'property', observable) },
      ]),
    ),
    actions: Object.fromEntries(
      actions.map((name) => [name, { forms: forms(thing, name, 'action') }]),
    ),
    events: Object.fromEntries(
      events.map((name) => [name, { forms: forms(thing, name, 'event') }]),
    ),
  }
}
