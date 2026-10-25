import { defineStore } from 'pinia'
import { shallowRef } from 'vue'

import { useThingMirror, type EventData } from './thing'

/** The focus drive, in µm: where it is, what it does, and its moves. `halt` passes the lock. */
export const useFocusStore = defineStore('focus', () => {
  const focus = useThingMirror('focus')
  const position = focus.property('position')
  const state = focus.property('state')
  const homed = focus.property('homed')
  const limits = focus.property('limits')
  const arrived = shallowRef<EventData<'focus', 'arrived'>>()
  focus.on('arrived', (at) => (arrived.value = at))

  return {
    fitted: focus.fitted,
    position,
    state,
    homed,
    limits,
    arrived,
    moveTo: (z: number, unidirectional = false) => focus.run('move_to', { z, unidirectional }),
    moveBy: (dz: number, unidirectional = false) => focus.run('move_by', { dz, unidirectional }),
    halt: () => focus.run('halt'),
  }
})
