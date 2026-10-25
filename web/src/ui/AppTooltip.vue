<script setup lang="ts">
// A short label beside its trigger (the default slot) while the pointer rests
// on it or it has keyboard focus, on Reka UI's Tooltip (ADR-0013); focus
// moved by code after a click doesn't show it. Escape hides it. It repeats, never replaces, the trigger's accessible name: a
// tooltip can't be reached by touch.
import {
  TooltipContent,
  TooltipPortal,
  TooltipProvider,
  TooltipRoot,
  TooltipTrigger,
} from 'reka-ui'

withDefaults(defineProps<{ text: string; side?: 'top' | 'right' | 'bottom' | 'left' }>(), {
  side: 'top',
})
</script>

<template>
  <!-- Each tooltip brings its provider, so none is needed above it. -->
  <TooltipProvider :delay-duration="500" ignore-non-keyboard-focus>
    <TooltipRoot>
      <TooltipTrigger as-child><slot /></TooltipTrigger>
      <TooltipPortal>
        <TooltipContent :side="side" :side-offset="4" class="app-tooltip">{{
          text
        }}</TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </TooltipProvider>
</template>

<style>
/* Not scoped: the tooltip is rendered at the end of <body>. Inverted colours, in both themes. */
.app-tooltip {
  z-index: var(--z-toast);
  max-width: 280px;
  padding: var(--space-1) var(--space-2);
  border-radius: var(--radius-sm);
  background: var(--color-text);
  color: var(--color-bg);
  font-size: var(--font-size-sm);
  line-height: var(--line-height-tight);
}
</style>
