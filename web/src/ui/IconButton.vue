<script setup lang="ts">
// A square button showing only an icon, such as the stage's arrow pad. Its
// `label` names it for assistive technology and is its tooltip. Other
// attributes and listeners go to the button.
import AppButton from './AppButton.vue'
import AppTooltip from './AppTooltip.vue'

defineOptions({ inheritAttrs: false })
withDefaults(
  defineProps<{
    /** The icon, as raw SVG text (see AppIcon). */
    icon: string
    label: string
    variant?: 'default' | 'primary' | 'danger' | 'ghost'
    size?: 'sm' | 'md'
  }>(),
  { variant: 'default', size: 'md' },
)
</script>

<template>
  <AppTooltip :text="label">
    <AppButton
      v-bind="$attrs"
      class="icon-button"
      :class="`icon-button--${size}`"
      :variant="variant"
      :size="size"
      :icon="icon"
      :aria-label="label"
    />
  </AppTooltip>
</template>

<style scoped>
.icon-button {
  width: var(--control-height-md);
  padding: 0;
}

.icon-button--sm {
  width: var(--control-height-sm);
}
</style>
