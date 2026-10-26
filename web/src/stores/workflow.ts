import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import { ApiError } from '@/api/wot/errors'
import type { Invocation } from '@/api/wot/invocation'

import { notifyFailure, useThingMirror, type EventData, type Input, type Output } from './thing'

/** A tile scan, as the `workflow` Thing takes it (ADR-0056). */
export type TileScanRequest = Input<'workflow', 'tile_scan'>
/** What one would take. */
export type ScanEstimate = Output<'workflow', 'estimate_tile_scan'>
/** A saved position, by its name: the server names every one. */
export type Position = Output<'workflow', 'save_position'>[number] & { name: string }

/**
 * The workflows: what runs and how far it has gone, the field of view, the saved positions, and
 * tile scans to estimate and start.
 */
export const useWorkflowStore = defineStore('workflow', () => {
  const workflow = useThingMirror('workflow')
  const state = workflow.property('state')
  const progress = workflow.property('progress')
  const mirrored = workflow.property('positions')
  const positions = computed(() => mirrored.value as Position[] | undefined)
  const field = workflow.property('field', { pollMs: 5_000 })
  const finished = shallowRef<EventData<'workflow', 'finished'>>()
  workflow.on('finished', (end) => (finished.value = end))
  /** The run started here, while it runs. */
  const running = shallowRef<Invocation>()

  /** What `request` would take, or why it couldn't run: without a toast, as a form changes. */
  async function estimate(request: TileScanRequest): Promise<ScanEstimate | string> {
    const consumed = workflow.thing.value
    if (!consumed) return 'The server has no workflows.'
    try {
      const invocation = await consumed.invokeAction('estimate_tile_scan', request)
      return await invocation.output<ScanEstimate>()
    } catch (failure) {
      return ApiError.fromFailure(failure).message
    }
  }

  /** Starts a tile scan and follows it: whether it started. Its failure is a toast. */
  async function startTileScan(request: TileScanRequest) {
    const consumed = workflow.thing.value
    if (!consumed) return false
    try {
      const invocation = await consumed.invokeAction('tile_scan', request)
      running.value = invocation
      invocation
        .output()
        .catch((failure) => notifyFailure(`The scan ${request.name}`, failure))
        .finally(() => (running.value = undefined))
      return true
    } catch (failure) {
      notifyFailure(`The scan ${request.name} didn't start`, failure)
      return false
    }
  }

  /** The saved positions after `changed`, which answers with them. */
  async function keep(changed: Promise<Output<'workflow', 'save_position'> | undefined>) {
    const list = await changed
    if (list) mirrored.value = list
    return list
  }

  return {
    fitted: workflow.fitted,
    state,
    progress,
    positions,
    field,
    finished,
    running,
    estimate,
    startTileScan,
    savePosition: (position: Input<'workflow', 'save_position'>) =>
      keep(workflow.run('save_position', position)),
    renamePosition: (name: string, to: string) =>
      keep(workflow.run('rename_position', { name, to })),
    removePosition: (name: string) => keep(workflow.run('remove_position', { name })),
  }
})
