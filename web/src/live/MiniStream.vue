<script setup lang="ts">
// A small live image, from the camera's ROI stream: for the Live page's
// picture-in-picture, and panels that need to see what the camera sees.
import LiveImage from './LiveImage.vue'

withDefaults(defineProps<{ thing?: string }>(), { thing: 'camera' })
</script>

<template>
  <LiveImage :thing="thing" stream="roi" class="mini-stream" />
</template>

<style scoped>
.mini-stream {
  flex: none;
  width: 100%;
  max-width: 20rem;
  aspect-ratio: 4 / 3;
  border-radius: var(--radius-md);
}
</style>
