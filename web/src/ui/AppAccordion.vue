<script setup lang="ts">
// Collapsible sections (AccordionSection), on Reka UI's Accordion (ADR-0013).
// Any number can be open; `v-model` lists the values of the open ones. ↑, ↓,
// Home and End move between the section titles.
import { AccordionRoot } from 'reka-ui'

const model = defineModel<string[]>({ default: () => [] })
</script>

<template>
  <AccordionRoot v-model="model" type="multiple" class="app-accordion">
    <slot />
  </AccordionRoot>
</template>

<style scoped>
.app-accordion {
  border-top: var(--border-width) solid var(--color-border);
}
</style>
