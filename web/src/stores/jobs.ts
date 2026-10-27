import { defineStore } from 'pinia'

import { useThingMirror, type Input, type Value } from './thing'

/** A job, as the `jobs` Thing lists it. */
export type JobInfo = Value<'jobs', 'running'>[number]
/** How a dataset is stitched (ADR-0064). */
export type StitchOptions = NonNullable<Input<'jobs', 'stitch'>['options']>

/**
 * The background jobs (ADR-0043): those waiting and running, followed as the server changes them;
 * cancelling one; a dataset's stitch; and whether a finished tile scan is stitched by itself.
 */
export const useJobsStore = defineStore('jobs', () => {
  const jobs = useThingMirror('jobs')
  const queue = jobs.property('queue')
  const running = jobs.property('running')
  const autoStitch = jobs.property('auto_stitch')

  return {
    fitted: jobs.fitted,
    queue,
    running,
    autoStitch,
    setAutoStitch: (on: boolean) => jobs.write('auto_stitch', on, autoStitch),
    cancel: (id: number) => jobs.run('cancel', { id }),
    /** Stitches dataset `dataset` into `image`, as a job: its id. */
    stitch: (dataset: string, image: string, options: StitchOptions) =>
      jobs.run('stitch', { dataset, image, options }),
  }
})
