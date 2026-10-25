<script setup lang="ts">
// A raised panel that groups related content, such as a gallery item or a
// block of settings.
</script>

<template>
  <div class="app-card">
    <slot />
  </div>
</template>

<style scoped>
.app-card {
  padding: var(--space-4);
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-surface-raised);
  box-shadow: var(--shadow-sm);
}
</style>
