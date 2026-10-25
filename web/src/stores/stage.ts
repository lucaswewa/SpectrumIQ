import { defineStore } from 'pinia'
import { shallowRef } from 'vue'

import { useThingMirror, type EventData, type Input } from './thing'

/** The XYZ stage, in µm: where it is, what it does, and its moves. `halt` passes the global lock. */
export const useStageStore = defineStore('stage', () => {
  const stage = useThingMirror('stage')
  const position = stage.property('position')
  const state = stage.property('state')
  const homed = stage.property('homed')
  const speed = stage.property('speed')
  const limits = stage.property('limits')
  const arrived = shallowRef<EventData<'stage', 'arrived'>>()
  const alarm = shallowRef<EventData<'stage', 'alarm'>>()
  stage.on('arrived', (at) => (arrived.value = at))
  stage.on('alarm', (why) => (alarm.value = why))

  return {
    fitted: stage.fitted,
    position,
    state,
    homed,
    speed,
    limits,
    arrived,
    alarm,
    setSpeed: (umPerS: number) => stage.write('speed', umPerS),
    moveTo: (to: Input<'stage', 'move_to'>) => stage.run('move_to', to),
    moveBy: (by: Partial<Input<'stage', 'move_by'>>) => stage.run('move_by', by),
    jog: (velocity: Partial<Input<'stage', 'jog'>>) => stage.run('jog', velocity),
    home: () => stage.run('home'),
    halt: () => stage.run('halt'),
  }
})
