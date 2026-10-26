import { defineStore } from 'pinia'
import { computed, shallowRef } from 'vue'

import { ApiError } from '@/api/wot/errors'
import type { Invocation } from '@/api/wot/invocation'

import {
  notify,
  notifyFailure,
  useThingMirror,
  type EventData,
  type Input,
  type Output,
} from './thing'

/** A tile scan, as the `workflow` Thing takes it (ADR-0056). */
export type TileScanRequest = Input<'workflow', 'tile_scan'>
/** What one would take. */
export type ScanEstimate = Output<'workflow', 'estimate_tile_scan'>
/** Any plan, as the `workflow` Thing's `run` takes it. */
export type PlanRequest = Input<'workflow', 'run'>
/** A run's end. */
export type Finished = EventData<'workflow', 'finished'>
/** The actions that run. */
const RUNS = ['run', 'tile_scan', 'z_stack', 'multi_position', 'time_lapse'] as const
/** A saved position, by its name: the server names every one. */
export type Position = Output<'workflow', 'save_position'>[number] & { name: string }

/**
 * The workflows: what runs, how far it has gone and the failed step it waits on, the field of
 * view, the saved positions, runs to estimate, start and control, and a toast as each ends.
 */
export const useWorkflowStore = defineStore('workflow', () => {
  const workflow = useThingMirror('workflow')
  const state = workflow.property('state')
  const progress = workflow.property('progress')
  const problem = workflow.property('problem')
  const mirrored = workflow.property('positions')
  const positions = computed(() => mirrored.value as Position[] | undefined)
  const field = workflow.property('field', { pollMs: 5_000 })
  const finished = shallowRef<Finished>()
  const acquired = shallowRef<EventData<'workflow', 'tile_acquired'>>()
  workflow.on('tile_acquired', (key) => (acquired.value = key))
  workflow.on('finished', (end) => {
    finished.value = end
    const frames = `${end.frames} frame${end.frames === 1 ? '' : 's'}`
    if (end.outcome === 'finished') notify('success', `${end.name} finished: ${frames}.`)
    else if (end.outcome === 'cancelled')
      notify('info', `${end.name} was cancelled: ${frames} kept.`)
    else notify('error', `${end.name} failed: ${end.error ?? 'no reason given'}.`)
  })
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

  /** Starts a run and follows it: whether it started. Its end is `finished`'s toast. */
  async function start(action: 'tile_scan' | 'run', request: TileScanRequest | PlanRequest) {
    const consumed = workflow.thing.value
    if (!consumed) return false
    try {
      const invocation = await consumed.invokeAction(action, request)
      running.value = invocation
      invocation
        .output<Finished>()
        .then((end) => (finished.value = end))
        .catch(() => {})
        .finally(() => (running.value = undefined))
      return true
    } catch (failure) {
      notifyFailure(`${request.name} didn't start`, failure)
      return false
    }
  }

  /** Cancels the run under way: the one started here, or whichever the server is running. */
  async function cancel() {
    const consumed = workflow.thing.value
    if (!consumed) return
    const ongoing = running.value
      ? [running.value]
      : (await Promise.all(RUNS.map((a) => consumed.ongoingInvocations(a).catch(() => [])))).flat()
    await Promise.all(ongoing.map((invocation) => invocation.cancel()))
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
    problem,
    positions,
    field,
    finished,
    acquired,
    running,
    estimate,
    startTileScan: (request: TileScanRequest) => start('tile_scan', request),
    startPlan: (request: PlanRequest) => start('run', request),
    cancel,
    pause: () => workflow.run('pause'),
    resume: () => workflow.run('resume'),
    retry: () => workflow.run('retry'),
    skip: () => workflow.run('skip'),
    savePosition: (position: Input<'workflow', 'save_position'>) =>
      keep(workflow.run('save_position', position)),
    renamePosition: (name: string, to: string) =>
      keep(workflow.run('rename_position', { name, to })),
    removePosition: (name: string) => keep(workflow.run('remove_position', { name })),
  }
})
