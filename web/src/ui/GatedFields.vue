<script setup lang="ts">
// Controls that need `need` (ADR-0068): disabled, with the reason above them, while the session
// may not do that; disabled, not hidden. The fieldset only disables: its children lay out as its
// parent's own.
import { useGate } from '@/auth/gate'
import type { Need } from '@/auth/session'

const props = defineProps<{ need?: Need }>()
const reason = useGate(() => props.need)
</script>

<template>
  <p v-if="reason" class="gated-fields__reason" role="note">{{ reason }}</p>
  <fieldset class="gated-fields" :disabled="reason !== undefined">
    <slot />
  </fieldset>
</template>

<style scoped>
.gated-fields {
  display: contents;
}

.gated-fields__reason {
  margin: 0 0 var(--space-3);
  color: var(--color-text-muted);
}
</style>
