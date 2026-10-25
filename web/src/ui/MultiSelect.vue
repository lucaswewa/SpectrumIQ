<script setup lang="ts" generic="T extends string | number">
// A drop-down list from which any number of options can be chosen, on Reka
// UI's Select (ADR-0013). `v-model` is the chosen values, in the options'
// order. The list stays open while options are picked, and the trigger
// shows the chosen labels, or `placeholder` when there are none. In a
// FormField it takes the field's id, description and error state.
import check from '@material-symbols/svg-400/outlined/check.svg?raw'
import chevron from '@material-symbols/svg-400/outlined/keyboard_arrow_down.svg?raw'
import {
  SelectContent,
  SelectIcon,
  SelectItem,
  SelectItemIndicator,
  SelectItemText,
  SelectPortal,
  SelectRoot,
  SelectTrigger,
  SelectViewport,
} from 'reka-ui'
import { computed, ref } from 'vue'

import AppIcon from './AppIcon.vue'
import { useFieldControl } from './field'
import { regionTheme } from './regionTheme'

const model = defineModel<T[]>({ default: () => [] })
const props = withDefaults(
  defineProps<{
    options: readonly { value: T; label: string; disabled?: boolean }[]
    placeholder?: string
    size?: 'sm' | 'md'
    invalid?: boolean
    disabled?: boolean
  }>(),
  { placeholder: undefined, size: 'md', invalid: false, disabled: false },
)

const field = useFieldControl(() => props.invalid)

/**
 * Keeps the chosen values in the options' order, whatever order they were
 * picked in. (Reka UI types the value as one option even with `multiple`.)
 */
function choose(values: unknown) {
  const chosen = values as T[]
  model.value = props.options
    .map((option) => option.value)
    .filter((value) => chosen.includes(value))
}

// The chosen labels, from the options. Reka UI's SelectValue shows the
// placeholder for a moment after the list closes while it finds them again.
const chosenLabels = computed(() =>
  props.options
    .filter((option) => model.value.includes(option.value))
    .map((option) => option.label)
    .join(', '),
)

const root = ref<HTMLElement>()
const theme = ref<string>()
function onOpen(open: boolean) {
  if (open) theme.value = regionTheme(root.value)
}
</script>

<template>
  <span ref="root" class="app-select">
    <SelectRoot
      :model-value="model"
      multiple
      :disabled="disabled"
      @update:model-value="choose"
      @update:open="onOpen"
    >
      <SelectTrigger
        :id="field.id"
        class="ui-input app-select__trigger"
        :class="`ui-input--${size}`"
        :aria-invalid="field.invalid.value || undefined"
        :aria-describedby="field.describedBy.value"
      >
        <span class="app-select__value">{{ chosenLabels || placeholder }}</span>
        <SelectIcon as-child>
          <AppIcon :svg="chevron" size="var(--icon-size-sm)" />
        </SelectIcon>
      </SelectTrigger>
      <SelectPortal>
        <SelectContent
          position="popper"
          :side-offset="2"
          :data-theme="theme"
          aria-multiselectable="true"
          class="app-select__list"
        >
          <SelectViewport>
            <SelectItem
              v-for="option in options"
              :key="option.value"
              :value="option.value"
              :disabled="option.disabled"
              class="app-select__option"
            >
              <SelectItemText>{{ option.label }}</SelectItemText>
              <SelectItemIndicator as-child>
                <AppIcon :svg="check" size="var(--icon-size-sm)" />
              </SelectItemIndicator>
            </SelectItem>
          </SelectViewport>
        </SelectContent>
      </SelectPortal>
    </SelectRoot>
  </span>
</template>

<style src="./input.css"></style>

<style src="./select.css"></style>
