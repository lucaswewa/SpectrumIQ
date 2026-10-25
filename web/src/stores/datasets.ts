import { defineStore } from 'pinia'
import { ref, shallowRef, watch } from 'vue'

import { useThingMirror, type EventData } from './thing'

/** A dataset, as the API lists it: its id, kind, name, date, user, raw frames and images. */
export type DatasetEntry = EventData<'datasets', 'created'>

/** The most datasets listed: the API's largest page. */
const LISTED = 500

/**
 * The datasets (ADR-0041, amended): the newest 500, read when the Thing is there and again when
 * one is made; their exports, and deleting one.
 */
export const useDatasetsStore = defineStore('datasets', () => {
  const datasets = useThingMirror('datasets')
  const list = shallowRef<DatasetEntry[]>()
  const total = ref(0)

  async function load() {
    const page = await datasets.run('summary', { offset: 0, limit: LISTED })
    if (page) [list.value, total.value] = [page.datasets, page.total]
  }
  watch(datasets.thing, (thing) => (thing ? void load() : (list.value = undefined)), {
    immediate: true,
  })
  datasets.on('created', () => void load())

  return {
    fitted: datasets.fitted,
    list,
    total,
    load,
    /** Exports image `image` as OME-TIFF, as a job: its id. */
    exportOmeTiff: (dataset: string, image: string) =>
      datasets.run('export_ome_tiff', { dataset, image }),
    /** Exports image `image`'s first plane as a static DZI pyramid, as a job: its id. */
    exportDzi: (dataset: string, image: string) => datasets.run('export_dzi', { dataset, image }),
    /** Deletes dataset `dataset`, then reads the list again. */
    async remove(dataset: string) {
      const deleted = await datasets.run('delete', { dataset })
      await load()
      return deleted
    },
  }
})
