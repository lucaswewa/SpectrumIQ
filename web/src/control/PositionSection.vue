<script setup lang="ts">
// Where the stage and the focus are, observed live, and what the stage is doing; fields to send
// them somewhere; the focus's fine steps; and Home, after a confirmation. STOP, in the toolbar,
// cancels any of it.
import focusUp from '@material-symbols/svg-400/outlined/add.svg?raw'
import focusDown from '@material-symbols/svg-400/outlined/remove.svg?raw'
import refreshIcon from '@material-symbols/svg-400/outlined/sync.svg?raw'
import { computed, ref } from 'vue'

import { useFocusStore } from '@/stores/focus'
import { useStageStore } from '@/stores/stage'
import type { Value } from '@/stores/thing'
import AppButton from '@/ui/AppButton.vue'
import IconButton from '@/ui/IconButton.vue'
import NumberField from '@/ui/NumberField.vue'
import { useConfirm } from '@/ui/overlays'

import { useNavigationPreferences } from './navigation'
import { useMotion } from './useJog'

const AXES = ['x', 'y', 'z'] as const
const STATES: Record<Exclude<Value<'stage', 'state'>, object>, string> = {
  idle: 'Still',
  moving: 'Moving',
  jogging: 'Jogging',
  homing: 'Homing',
  holding: 'Holding',
  disconnected: 'Disconnected',
}

const stage = useStageStore()
const focus = useFocusStore()
const preferences = useNavigationPreferences()
const motion = useMotion()
const confirm = useConfirm()
const target = ref({ x: 0, y: 0, z: 0, focus: 0 })

const um = (value: number | undefined) => (value === undefined ? '…' : `${value.toFixed(1)} µm`)
const state = computed(() => {
  const now = stage.state
  if (now === undefined) return 'Unknown'
  if (typeof now === 'object') return `Alarm ${now.alarm}: motion is refused`
  return STATES[now]
})

/** Fills the fields in with where the stage and the focus are. */
function refresh() {
  if (stage.position) target.value = { ...stage.position, focus: target.value.focus }
  if (focus.position !== undefined) target.value.focus = focus.position
}

const moveStage = () => stage.moveTo({ x: target.value.x, y: target.value.y, z: target.value.z })
const moveFocus = () => focus.moveTo(target.value.focus)
const stepFocus = (sign: 1 | -1) =>
  motion.focus.add({ x: 0, y: 0, z: sign * preferences.steps.focus })

async function home() {
  const confirmed = await confirm({
    title: 'Home the stage?',
    message:
      'Each axis travels to its reference switch, which can be far: lower the stage or take the sample away if anything could collide.',
    confirmLabel: 'Home',
  })
  if (confirmed) await stage.home()
}
</script>

<template>
  <div class="position-section">
    <dl class="position-section__readouts">
      <template v-for="axis in AXES" :key="axis">
        <dt>{{ axis === 'z' ? 'Coarse z' : axis }}</dt>
        <dd :data-test="`position-${axis}`">{{ um(stage.position?.[axis]) }}</dd>
      </template>
      <dt>Focus</dt>
      <dd data-test="position-focus">{{ um(focus.position) }}</dd>
      <dt>Stage</dt>
      <dd role="status" :class="{ 'position-section__alarm': typeof stage.state === 'object' }">
        {{ state }}{{ stage.homed === false ? ', not homed' : '' }}
      </dd>
    </dl>
    <div class="position-section__focus" role="group" aria-label="Focus">
      <IconButton :icon="focusDown" label="Focus down" @click="stepFocus(-1)" />
      <span>{{ preferences.steps.focus }} µm a step, or the wheel on the image</span>
      <IconButton :icon="focusUp" label="Focus up" @click="stepFocus(1)" />
    </div>
    <div class="position-section__row">
      <label v-for="axis in AXES" :key="axis" class="position-section__axis">
        <span>{{ axis }}</span>
        <NumberField v-model="target[axis]" size="sm" :step="1" @submit="moveStage" />
      </label>
      <label class="position-section__axis">
        <span>f</span>
        <NumberField v-model="target.focus" size="sm" :step="0.1" @submit="moveFocus" />
      </label>
    </div>
    <div class="position-section__actions">
      <IconButton
        :icon="refreshIcon"
        label="Fill in where the stage and the focus are"
        size="sm"
        variant="ghost"
        @click="refresh"
      />
      <AppButton size="sm" :disabled="!stage.fitted" @click="moveStage">Move stage</AppButton>
      <AppButton size="sm" :disabled="!focus.fitted" @click="moveFocus">Move focus</AppButton>
      <AppButton size="sm" :disabled="!stage.fitted" @click="home">Home…</AppButton>
    </div>
  </div>
</template>

<style scoped>
.position-section {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.position-section__readouts {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-3);
  margin: 0;
}

.position-section__readouts dt {
  color: var(--color-text-muted);
}

.position-section__readouts dd {
  margin: 0;
  font-variant-numeric: tabular-nums;
}

.position-section__alarm {
  color: var(--color-danger);
  font-weight: var(--font-weight-semibold);
}

.position-section__focus {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  justify-content: space-between;
  color: var(--color-text-muted);
  font-size: var(--font-size-sm);
}

.position-section__row {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: var(--space-2);
}

.position-section__axis {
  display: grid;
  grid-template-columns: var(--space-4) 1fr;
  gap: var(--space-2);
  align-items: center;
}

.position-section__actions {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
  align-items: center;
}
</style>
