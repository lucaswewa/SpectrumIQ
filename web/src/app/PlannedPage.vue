<script setup lang="ts">
// The page of a destination that isn't built yet: its name, and the milestone that builds it.
import { computed } from 'vue'

import MainView from '@/ui/MainView.vue'

import { DESTINATIONS } from './navigation'

const props = defineProps<{ destination: string }>()

const shown = computed(() =>
  DESTINATIONS.find((destination) => destination.id === props.destination),
)
</script>

<template>
  <MainView class="planned-page">
    <h1>{{ shown?.label }}</h1>
    <p class="planned-page__note">
      This page comes in {{ shown?.planned ? `milestone ${shown.planned}` : 'a later milestone' }}
      of the plan.
    </p>
  </MainView>
</template>

<style scoped>
.planned-page h1 {
  margin: 0 0 var(--space-2);
  font-size: var(--font-size-xl);
}

.planned-page__note {
  margin: 0;
  color: var(--color-text-muted);
}
</style>
