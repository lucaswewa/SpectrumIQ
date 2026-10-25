<script setup lang="ts">
// The connection to the server, at the foot of the rail: connected, connecting, or lost, with
// the server's version, and a way to try again at once.
import { computed } from 'vue'

import { useConnectionStore } from '@/stores/connection'

const connection = useConnectionStore()

const LABELS = {
  idle: 'Not connected',
  connecting: 'Connecting…',
  connected: 'Connected',
  lost: 'Lost',
  reconnecting: 'Reconnecting…',
} as const

const title = computed(() => {
  const health = connection.health
  return connection.connected && health
    ? `Connected to SpectrumIQ ${health.version}, with ${health.camera}`
    : LABELS[connection.state]
})
</script>

<template>
  <div class="connection" :class="`connection--${connection.state}`" role="status" :title="title">
    <span class="connection__dot" aria-hidden="true" />
    <span class="connection__label">{{ LABELS[connection.state] }}</span>
    <button
      v-if="connection.state === 'lost'"
      type="button"
      class="connection__retry"
      @click="connection.retry()"
    >
      Retry
    </button>
  </div>
</template>

<style scoped>
.connection {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  align-items: center;
  padding: var(--space-2) var(--space-1);
  border-top: var(--border-width) solid var(--color-border);
  color: var(--color-rail-text);
  font-size: var(--font-size-sm);
  text-align: center;
}

.connection__dot {
  width: var(--space-2);
  height: var(--space-2);
  border-radius: var(--radius-full);
  background: var(--color-text-disabled);
}

.connection--connected .connection__dot {
  background: var(--color-success);
}

.connection--lost .connection__dot {
  background: var(--color-danger);
}

.connection--connecting .connection__dot,
.connection--reconnecting .connection__dot {
  background: var(--color-warning);
}

.connection__retry {
  padding: 0 var(--space-1);
  border: var(--border-width) solid var(--color-border-strong);
  border-radius: var(--radius-sm);
  background: none;
  color: inherit;
  font: inherit;
  cursor: pointer;
}
</style>
