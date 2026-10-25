<script setup lang="ts">
// What went wrong, from an error or a server's error response (see
// describeError): its heading and message, and each invalid input of a 422.
import { computed } from 'vue'

import { describeError } from '@/api/wot/errors'

const props = defineProps<{ error: unknown }>()

const described = computed(() => describeError(props.error))
</script>

<template>
  <div class="error-details">
    <p v-if="described.title" class="error-details__title">{{ described.title }}</p>
    <p v-if="described.message" class="error-details__message">{{ described.message }}</p>
    <ul v-if="described.issues.length" class="error-details__issues">
      <li v-for="(issue, index) in described.issues" :key="index">
        <code v-if="issue.field" class="error-details__field">{{ issue.field }}</code>
        {{ issue.message }}
      </li>
    </ul>
    <pre v-if="described.json" class="error-details__json">{{ described.json }}</pre>
  </div>
</template>

<style scoped>
.error-details {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  font-size: var(--font-size-sm);
}

.error-details p,
.error-details ul,
.error-details pre {
  margin: 0;
}

.error-details__title {
  font-weight: var(--font-weight-semibold);
}

.error-details__issues {
  padding-left: var(--space-4);
}

.error-details__field,
.error-details__json {
  font-family: var(--font-family-mono);
  font-size: var(--font-size-xs);
}

.error-details__field {
  margin-right: var(--space-1);
  padding: 0 var(--space-1);
  border-radius: var(--radius-sm);
  background: var(--color-surface-sunken);
}

.error-details__json {
  max-height: 12em;
  overflow: auto;
  white-space: pre-wrap;
}
</style>
