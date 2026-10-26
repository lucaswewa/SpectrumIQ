import { defineStore } from 'pinia'
import { computed, ref, shallowRef, watch } from 'vue'

import { useCalibrationStore } from '@/stores/calibration'
import { usePeripheralsStore } from '@/stores/peripherals'

import { allNeeded, FINAL, neededFor, WELCOME, type WizardTask } from './tasks'

/**
 * Whether the calibration wizard is open, and with which tasks (ADR-0051). It opens by itself,
 * with a welcome, once in each page's life for each objective that comes into use lacking a
 * calibration: the objective at the turret, or the only one configured. The Calibration page
 * opens it with the tasks it chooses.
 */
export const useCalibrationWizard = defineStore('calibration-wizard', () => {
  const calibration = useCalibrationStore()
  const peripherals = usePeripheralsStore()
  const open = ref(false)
  const tasks = shallowRef<WizardTask[]>([])
  const offered = new Set<number>()

  /** Opens with `chosen`, after a welcome if `welcome`: not at all, if there is nothing. */
  function start(chosen: WizardTask[], welcome = false) {
    if (chosen.length === 0) return
    tasks.value = [...(welcome ? [WELCOME] : []), ...chosen, FINAL]
    open.value = true
  }

  /** The objective in use, with its calibrations. */
  const inUse = computed(() => {
    const status = calibration.status
    const current = peripherals.objectives.current
    if (current) return status?.find((entry) => entry.objective === current.id)
    return status?.length === 1 ? status[0] : undefined
  })
  watch(
    inUse,
    (entry) => {
      if (!entry || open.value || offered.has(entry.objective)) return
      offered.add(entry.objective)
      start(neededFor(entry), true)
    },
    { immediate: true },
  )

  return {
    open,
    tasks,
    inUse,
    start,
    /** Opens with everything every objective lacks. */
    openNeeded: () => start(allNeeded(calibration.status ?? [])),
  }
})
