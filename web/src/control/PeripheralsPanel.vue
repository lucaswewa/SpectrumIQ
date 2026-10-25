<script setup lang="ts">
// The peripherals: the channel, each light's level, the filter, the objective (after a
// confirmation, with its parfocal offset), and the modality. Each shows only if it is fitted.
import { computed } from 'vue'

import { useImagingStore } from '@/stores/imaging'
import { usePeripheralsStore } from '@/stores/peripherals'
import type { Value } from '@/stores/thing'
import AppSelect from '@/ui/AppSelect.vue'
import AppSlider from '@/ui/AppSlider.vue'
import FormField from '@/ui/FormField.vue'
import { useConfirm } from '@/ui/overlays'

type Mode = Value<'modality', 'modes'>[number]

const MODES: Record<Mode, string> = {
  widefield: 'Widefield',
  light_field: 'Light field',
  confocal: 'Confocal',
  structured_illumination: 'Structured illumination',
}

const imaging = useImagingStore()
const peripherals = usePeripheralsStore()
const { lights, filters, objectives, modes } = peripherals
const confirm = useConfirm()

const channel = computed({
  get: () => imaging.channel ?? undefined,
  set: (id) => id !== undefined && void imaging.selectChannel(id),
})
const channelOptions = computed(() =>
  (imaging.channels ?? []).map((each) => ({ value: each.id, label: each.name })),
)

/** Sends each light's newest level, one at a time: a slider being dragged sends the latest. */
const pending = new Map<string, number>()
const sending = new Set<string>()
async function setLevel(source: string, percent: number) {
  pending.set(source, percent / 100)
  if (sending.has(source)) return
  sending.add(source)
  for (let level = pending.get(source); level !== undefined; level = pending.get(source)) {
    pending.delete(source)
    await peripherals.setLevel(source, level)
  }
  sending.delete(source)
}

const filter = computed({
  get: () => filters.position ?? undefined,
  set: (position) => position !== undefined && void peripherals.selectFilter(position),
})
const filterOptions = computed(() =>
  (filters.names ?? []).map((name, position) => ({ value: position, label: name })),
)

const objective = computed({
  get: () => objectives.position ?? undefined,
  set: (position) => position !== undefined && void changeObjective(position),
})
const objectiveOptions = computed(() =>
  (objectives.configured ?? []).flatMap((each) =>
    each.turret_position === null || each.turret_position === undefined
      ? []
      : [{ value: each.turret_position, label: each.name }],
  ),
)

/** The objective at turret `position`, after a confirmation: the turret turns under the sample. */
async function changeObjective(position: number) {
  const next = objectives.configured?.find((each) => each.turret_position === position)
  if (!next) return
  const shift = (next.parfocal_offset ?? 0) - (objectives.current?.parfocal_offset ?? 0)
  const parfocal =
    shift === 0
      ? 'It is parfocal with this one.'
      : `Its focus is ${Math.abs(shift)} µm ${shift > 0 ? 'above' : 'below'} this one's (parfocal offsets): the focus isn't moved, so refocus after.`
  const immersion = next.immersion && next.immersion !== 'air' ? ` It takes ${next.immersion}.` : ''
  const confirmed = await confirm({
    title: `Change to ${next.name}?`,
    message: `The turret turns under the sample. ${parfocal}${immersion}`,
    confirmLabel: 'Change',
  })
  if (confirmed) await peripherals.selectObjective(position)
}

const mode = computed({
  get: () => modes.modality?.mode,
  set: (next) => next !== undefined && void peripherals.setModality({ mode: next }),
})
const modeOptions = computed(() =>
  (modes.offered ?? []).map((each) => ({ value: each, label: MODES[each] })),
)
</script>

<template>
  <div class="peripherals-panel">
    <FormField v-if="imaging.fitted" label="Channel">
      <AppSelect v-model="channel" :options="channelOptions" placeholder="None" size="sm" />
    </FormField>
    <template v-if="lights.fitted">
      <div v-for="(source, index) in lights.sources" :key="source" class="peripherals-panel__light">
        <span>{{ source }}</span>
        <AppSlider
          :model-value="Math.round((lights.levels?.[index] ?? 0) * 100)"
          :label="`${source} level`"
          @update:model-value="(percent) => setLevel(source, percent)"
        />
        <span class="peripherals-panel__level">
          {{ Math.round((lights.levels?.[index] ?? 0) * 100) }} %
        </span>
      </div>
    </template>
    <FormField v-if="filters.fitted" label="Filter">
      <AppSelect v-model="filter" :options="filterOptions" placeholder="Moving…" size="sm" />
    </FormField>
    <FormField v-if="objectives.fitted" label="Objective">
      <AppSelect v-model="objective" :options="objectiveOptions" placeholder="None" size="sm" />
    </FormField>
    <FormField v-if="modes.fitted" label="Modality">
      <AppSelect v-model="mode" :options="modeOptions" placeholder="Moving…" size="sm" />
    </FormField>
  </div>
</template>

<style scoped>
.peripherals-panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.peripherals-panel__light {
  display: grid;
  grid-template-columns: 4rem 1fr 3rem;
  gap: var(--space-2);
  align-items: center;
  font-size: var(--font-size-sm);
}

.peripherals-panel__level {
  text-align: end;
  font-variant-numeric: tabular-nums;
}
</style>
