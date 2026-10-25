import { computed, inject, type InjectionKey } from 'vue'

import { DESTINATIONS } from './navigation'

/**
 * Whether the connected instrument offers a Thing, by name. It may read
 * reactive state (such as the connection's Thing Descriptions): the rail
 * updates when that state changes.
 */
export type ThingAvailability = (thing: string) => boolean

/**
 * The key under which the app provides Thing availability. The connection
 * store (P3.2) provides it from the Thing Descriptions. Until something
 * is provided, every Thing counts as available, so every tab shows.
 */
export const thingAvailabilityKey: InjectionKey<ThingAvailability> = Symbol('thing availability')

const everyThing: ThingAvailability = () => true

/** The destinations whose required Things are all available, in rail order. */
export function useAvailableDestinations() {
  const available = inject(thingAvailabilityKey, everyThing)
  return computed(() =>
    DESTINATIONS.filter((destination) => destination.requires.every((thing) => available(thing))),
  )
}
