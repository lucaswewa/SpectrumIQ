<script setup lang="ts">
// One section of an AppAccordion: a title that opens and closes it, and the
// default slot as its content, which is only rendered while open. The title
// is a button in an <h3>, and the content a region named by the title.
import chevron from '@material-symbols/svg-400/outlined/keyboard_arrow_down.svg?raw'
import { AccordionContent, AccordionHeader, AccordionItem, AccordionTrigger } from 'reka-ui'
import { onMounted, ref, useTemplateRef } from 'vue'

import AppIcon from './AppIcon.vue'

defineProps<{ value: string; title: string; disabled?: boolean }>()

// Reka UI 2.10 renders the title's aria-controls before the content has its
// id, and never updates it, so it stays empty. Point it at the content once
// both exist.
const content = useTemplateRef<{ $el: HTMLElement }>('content')
const contentId = ref<string>()
onMounted(() => (contentId.value = content.value?.$el.id))
</script>

<template>
  <AccordionItem :value="value" :disabled="disabled" class="accordion-section">
    <AccordionHeader class="accordion-section__header">
      <AccordionTrigger class="accordion-section__trigger" :aria-controls="contentId">
        {{ title }}
        <AppIcon :svg="chevron" size="var(--icon-size-sm)" class="accordion-section__chevron" />
      </AccordionTrigger>
    </AccordionHeader>
    <AccordionContent ref="content" class="accordion-section__content">
      <slot />
    </AccordionContent>
  </AccordionItem>
</template>

<style scoped>
.accordion-section {
  border-bottom: var(--border-width) solid var(--color-border);
}

/* An <h3>, but in the pane's text style, not a heading's. */
.accordion-section__header {
  margin: 0;
  font: inherit;
}

.accordion-section__trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  height: var(--control-height-md);
  padding: 0 var(--space-1);
  border: none;
  background: none;
  color: var(--color-text);
  font: inherit;
  font-weight: var(--font-weight-medium);
  text-align: left;
  cursor: pointer;
}

.accordion-section__trigger:hover:not(:disabled) {
  background: color-mix(in srgb, currentcolor 6%, transparent);
}

.accordion-section__trigger:disabled {
  color: var(--color-text-disabled);
  cursor: not-allowed;
}

.accordion-section__trigger:focus-visible {
  outline-offset: calc(-1 * var(--focus-ring-width));
}

.accordion-section__chevron {
  transition: transform var(--duration-normal) var(--easing-standard);
}

.accordion-section__trigger[data-state='open'] .accordion-section__chevron {
  transform: rotate(180deg);
}

.accordion-section__content {
  padding: var(--space-1) var(--space-1) var(--space-3);
}
</style>
