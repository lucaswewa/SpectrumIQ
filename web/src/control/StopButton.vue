<script setup lang="ts">
// STOP: halts the stage and the focus, and forgets every move still to come (ADR-0039). Never
// disabled, whatever is moving or busy: halting passes the global lock, and halting nothing is
// harmless. Only a viewer may not stop: disabled for one, with why (ADR-0068). Esc does the same
// from anywhere on the page.
import stop from '@material-symbols/svg-400/outlined/stop_circle-fill.svg?raw'

import { useGate } from '@/auth/gate'
import AppButton from '@/ui/AppButton.vue'

import { useMotion } from './useJog'

const motion = useMotion()
const refused = useGate(() => 'process')
</script>

<template>
  <AppButton
    class="stop-button"
    variant="danger"
    :icon="stop"
    aria-keyshortcuts="Escape"
    :disabled="refused !== undefined"
    :title="refused ?? 'Halt the stage and the focus (Esc)'"
    @click="motion.stop()"
  >
    STOP
  </AppButton>
</template>

<style scoped>
/* Filled, unlike other danger buttons, so it is found at once: in the page's background colour,
   which contrasts with the danger colour in either theme. */
.stop-button.app-button--danger {
  --button-bg: var(--color-danger);
  --button-text: var(--color-bg);
  --button-hover-bg: color-mix(in srgb, var(--color-danger) 85%, var(--color-text));

  font-weight: var(--font-weight-semibold);
}
</style>
