<script setup lang="ts">
// The title of a section of a page or pane, as the reference's: large, light
// and in the heading colour, whatever its level in the outline. Buttons in
// the `actions` slot sit at its right.
withDefaults(defineProps<{ level?: 2 | 3 | 4 }>(), { level: 2 })
</script>

<template>
  <div class="section-heading">
    <component :is="`h${level}`" class="section-heading__title"><slot /></component>
    <div v-if="$slots.actions" class="section-heading__actions"><slot name="actions" /></div>
  </div>
</template>

<style scoped>
.section-heading {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  justify-content: space-between;
  margin-bottom: var(--space-3);
}

.section-heading__title {
  margin: 0;
  color: var(--color-text-heading);
  font-family: var(--font-family-display);
  font-size: var(--font-size-2xl);
  font-weight: var(--font-weight-light);
  line-height: var(--line-height-tight);
}

.section-heading__actions {
  display: flex;
  gap: var(--space-1);
}
</style>
