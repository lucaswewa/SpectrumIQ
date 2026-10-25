<script setup lang="ts">
// A number input without spinner buttons (a `spinbutton`).
//
// `v-model` changes only when an edit is committed, so it can be bound to a
// setting on the microscope directly: Enter commits and emits `submit` (even
// if the value is unchanged), and leaving the field commits. ↑ and ↓ step the
// value and Escape abandons the edit. Text that isn't a number within
// `min`–`max` and on the `step` grid (counted from `min`, or 0) marks the
// field invalid and is never committed; leaving the field abandons it.
// While no edit is pending, the field follows changes to the model.
import { computed, ref, watch } from 'vue'

import { useFieldControl } from './field'

const model = defineModel<number>({ required: true })
const props = withDefaults(
  defineProps<{
    min?: number
    max?: number
    /** The grid of allowed values, and what ↑ and ↓ add. Any value is allowed without it. */
    step?: number
    size?: 'sm' | 'md'
    invalid?: boolean
  }>(),
  { min: undefined, max: undefined, step: undefined, size: 'md', invalid: false },
)
const emit = defineEmits<{ submit: [value: number] }>()

const draft = ref(String(model.value))
const editing = ref(false)

watch(model, (value) => {
  if (!editing.value) draft.value = String(value)
})

const NUMBER = /^[+-]?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?$/i

/** The draft as an acceptable value, or null. */
const value = computed(() => {
  const text = draft.value.trim()
  if (!NUMBER.test(text)) return null
  const number = Number(text)
  if (props.min !== undefined && number < props.min) return null
  if (props.max !== undefined && number > props.max) return null
  if (props.step !== undefined && !Number.isInteger(stepsTo(number, props.step))) return null
  return number
})

/** How many steps `number` is from the grid's origin: a whole number if it is on the grid. */
function stepsTo(number: number, step: number) {
  const steps = (number - (props.min ?? 0)) / step
  return Math.abs(steps - Math.round(steps)) < 1e-9 ? Math.round(steps) : steps
}

const field = useFieldControl(() => props.invalid || value.value === null)

function commit(submit: boolean) {
  const committed = value.value
  if (committed === null) return
  editing.value = false
  draft.value = String(committed)
  model.value = committed
  if (submit) emit('submit', committed)
}

function abandon() {
  editing.value = false
  draft.value = String(model.value)
}

/** Moves to the next value on the grid up or down (by 1 without a step), within min–max. */
function stepBy(direction: 1 | -1) {
  const current = value.value ?? model.value
  let next = current + direction
  if (props.step !== undefined) {
    const steps = stepsTo(current, props.step)
    const to = direction > 0 ? Math.floor(steps) + 1 : Math.ceil(steps) - 1
    next = (props.min ?? 0) + to * props.step
  }
  if (props.min !== undefined) next = Math.max(next, props.min)
  if (props.max !== undefined) next = Math.min(next, props.max)
  // Remove floating-point noise such as 0.30000000000000004.
  draft.value = String(Number(next.toPrecision(12)))
  editing.value = true
}

function onKeydown(event: KeyboardEvent) {
  if (event.isComposing) return
  if (event.key === 'Enter') commit(true)
  else if (event.key === 'Escape') abandon()
  else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
    event.preventDefault()
    stepBy(event.key === 'ArrowUp' ? 1 : -1)
  }
}

function onBlur() {
  if (!editing.value) return
  if (value.value === null) abandon()
  else commit(false)
}
</script>

<template>
  <input
    :id="field.id"
    v-model="draft"
    type="text"
    inputmode="decimal"
    autocomplete="off"
    role="spinbutton"
    class="ui-input"
    :class="`ui-input--${size}`"
    :aria-valuenow="value ?? undefined"
    :aria-valuemin="min"
    :aria-valuemax="max"
    :aria-invalid="field.invalid.value || undefined"
    :aria-describedby="field.describedBy.value"
    @input="editing = true"
    @keydown="onKeydown"
    @blur="onBlur"
  />
</template>

<style src="./input.css"></style>
