import { defineStore } from 'pinia'
import { watch } from 'vue'

import { useThingMirror, type Input } from './thing'

/** The fitted peripherals: illumination, filter wheel, objective turret and modality unit. */
export const usePeripheralsStore = defineStore('peripherals', () => {
  const illumination = useThingMirror('illumination')
  const filterWheel = useThingMirror('filter_wheel')
  const objective = useThingMirror('objective')
  const modality = useThingMirror('modality')

  const lights = {
    fitted: illumination.fitted,
    sources: illumination.property('sources'),
    levels: illumination.property('levels'),
  }
  const filters = {
    fitted: filterWheel.fitted,
    names: filterWheel.property('filters'),
    position: filterWheel.property('position'),
    moving: filterWheel.property('moving'),
  }
  const objectives = {
    fitted: objective.fitted,
    configured: objective.property('objectives'),
    position: objective.property('position'),
    moving: objective.property('moving'),
    current: objective.property('current'),
  }
  // The objective in place is a functional property: read it again after each move.
  watch(objectives.position, () => void objective.read('current', objectives.current))
  const modes = {
    fitted: modality.fitted,
    offered: modality.property('modes'),
    patterns: modality.property('patterns'),
    modality: modality.property('modality'),
    moving: modality.property('moving'),
  }

  return {
    lights,
    filters,
    objectives,
    modes,
    setLevel: (source: string, level: number) => illumination.run('set_level', { source, level }),
    selectFilter: (position: number) => filterWheel.run('select', { position }),
    selectObjective: (position: number) => objective.run('select', { position }),
    setModality: (next: Input<'modality', 'set_modality'>) => modality.run('set_modality', next),
  }
})
