<script setup lang="ts">
// A one-line text input. `v-model` follows every keystroke, like a native
// input's; Enter emits `submit`. In a FormField it takes the field's id,
// description and error state. Other attributes, such as `placeholder` or
// `disabled`, go to the <input>.
import { useFieldControl } from './field'

const model = defineModel<string>({ default: '' })
const props = withDefaults(defineProps<{ size?: 'sm' | 'md'; invalid?: boolean }>(), {
  size: 'md',
  invalid: false,
})
const emit = defineEmits<{ submit: [value: string] }>()

const field = useFieldControl(() => props.invalid)

function onEnter(event: KeyboardEvent) {
  // Enter that confirms an input method's composition isn't a submission.
  if (!event.isComposing) emit('submit', model.value)
}
</script>

<template>
  <input
    :id="field.id"
    v-model="model"
    type="text"
    class="ui-input"
    :class="`ui-input--${size}`"
    :aria-invalid="field.invalid.value || undefined"
    :aria-describedby="field.describedBy.value"
    @keydown.enter="onEnter"
  />
</template>

<style src="./input.css"></style>
