<script setup lang="ts" generic="T extends string | number">
// A drop-down list of options, on Reka UI's Select (ADR-0013), which handles
// the keyboard, typeahead, focus and ARIA. `v-model` is the chosen option's
// value; until one is chosen, `placeholder` shows. In a FormField it takes
// the field's id, description and error state.
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
  SelectValue,
  SelectViewport,
} from 'reka-ui'
import { ref } from 'vue'

import AppIcon from './AppIcon.vue'
import { useFieldControl } from './field'
import { regionTheme } from './regionTheme'

const model = defineModel<T>()
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

// The list is moved to <body> so no scrolling pane clips it, in the theme of
// its trigger's region.
const root = ref<HTMLElement>()
const theme = ref<string>()
function onOpen(open: boolean) {
  if (open) theme.value = regionTheme(root.value)
}
</script>

<template>
  <span ref="root" class="app-select">
    <SelectRoot v-model="model" :disabled="disabled" @update:open="onOpen">
      <SelectTrigger
        :id="field.id"
        class="ui-input app-select__trigger"
        :class="`ui-input--${size}`"
        :aria-invalid="field.invalid.value || undefined"
        :aria-describedby="field.describedBy.value"
      >
        <SelectValue :placeholder="placeholder" class="app-select__value" />
        <SelectIcon as-child>
          <AppIcon :svg="chevron" size="var(--icon-size-sm)" />
        </SelectIcon>
      </SelectTrigger>
      <SelectPortal>
        <SelectContent
          position="popper"
          :side-offset="2"
          :data-theme="theme"
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
