<script setup lang="ts">
import { onScopeDispose } from 'vue'
import { RouterView } from 'vue-router'

import CalibrationWizard from '@/calibration/CalibrationWizard.vue'
import { bindToaster } from '@/stores/thing'
import { useToast } from '@/ui/overlays'

import { useAvailableDestinations } from './availability'
import NavRail from './NavRail.vue'
import ShortcutHelp from './ShortcutHelp.vue'
import { useTabCycling } from './useTabCycling'

const destinations = useAvailableDestinations()
useTabCycling(destinations)
// The stores' failures appear as toasts.
bindToaster(useToast())
onScopeDispose(() => bindToaster(undefined))
</script>

<template>
  <div class="shell">
    <NavRail :destinations="destinations" />
    <div class="shell__content">
      <p class="shell__narrow-notice" role="note">
        This interface is designed for a wide, landscape window. Some controls may not fit until it
        is wider.
      </p>
      <RouterView />
    </div>
    <ShortcutHelp />
    <CalibrationWizard />
  </div>
</template>

<style scoped>
.shell {
  display: flex;
  height: 100%;
}

.shell__content {
  position: relative;
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}

.shell__narrow-notice {
  display: none;
  margin: 0;
  padding: var(--space-2) var(--space-4);
  border-bottom: var(--border-width) solid var(--color-border);
  background: var(--color-surface-sunken);
  color: var(--color-warning);
}

@media (width < 900px), (orientation: portrait) {
  .shell__narrow-notice {
    display: block;
  }
}
</style>
