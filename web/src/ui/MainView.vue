<script setup lang="ts">
// The main area of a page, beside an optional control pane: it takes the
// remaining width. With `backdrop`, it is the dark-or-light surround of the
// live image.
withDefaults(defineProps<{ backdrop?: boolean }>(), { backdrop: false })
</script>

<template>
  <section class="main-view" :class="{ 'main-view--backdrop': backdrop }">
    <slot />
  </section>
</template>

<style scoped>
.main-view {
  flex: 1;
  min-width: 0;
  height: 100%;
  padding: var(--space-6);
  overflow: auto;
}

.main-view--backdrop {
  background: var(--color-image-backdrop);
}
</style>
