<script setup lang="ts">
// The d-pad and the coarse-Z buttons. A press moves a step, and holding keeps
// moving until the button is released, wherever the pointer has gone: the
// button captures it.
import left from '@material-symbols/svg-400/outlined/arrow_back.svg?raw'
import down from '@material-symbols/svg-400/outlined/arrow_downward.svg?raw'
import right from '@material-symbols/svg-400/outlined/arrow_forward.svg?raw'
import up from '@material-symbols/svg-400/outlined/arrow_upward.svg?raw'
import zDown from '@material-symbols/svg-400/outlined/keyboard_double_arrow_down.svg?raw'
import zUp from '@material-symbols/svg-400/outlined/keyboard_double_arrow_up.svg?raw'

import IconButton from '@/ui/IconButton.vue'

import type { Axes } from './jog'
import { DOWN, LEFT, RIGHT, UP, useMotion, Z_DOWN, Z_UP } from './useJog'

const { controller } = useMotion()

const BUTTONS: { id: string; label: string; icon: string; direction: Axes }[] = [
  { id: 'up', label: 'Move up', icon: up, direction: UP },
  { id: 'left', label: 'Move left', icon: left, direction: LEFT },
  { id: 'right', label: 'Move right', icon: right, direction: RIGHT },
  { id: 'down', label: 'Move down', icon: down, direction: DOWN },
  { id: 'z-down', label: 'Coarse Z down', icon: zDown, direction: Z_DOWN },
  { id: 'z-up', label: 'Coarse Z up', icon: zUp, direction: Z_UP },
]

function press(event: PointerEvent, button: (typeof BUTTONS)[number]) {
  if (event.button !== 0) return
  ;(event.currentTarget as Element).setPointerCapture?.(event.pointerId)
  controller.press(`pointer:${button.id}`, button.direction)
}

function release(button: (typeof BUTTONS)[number]) {
  controller.release(`pointer:${button.id}`)
}

/** Enter or Space on a focused button moves one step. */
function click(event: MouseEvent, button: (typeof BUTTONS)[number]) {
  if (event.detail !== 0) return
  controller.press(`keyboard:${button.id}`, button.direction)
  controller.release(`keyboard:${button.id}`)
}
</script>

<template>
  <div class="direction-pad" role="group" aria-label="Move the stage">
    <IconButton
      v-for="button in BUTTONS"
      :key="button.id"
      :class="`direction-pad__${button.id}`"
      :icon="button.icon"
      :label="button.label"
      variant="primary"
      @pointerdown="press($event, button)"
      @pointerup="release(button)"
      @pointercancel="release(button)"
      @lostpointercapture="release(button)"
      @click="click($event, button)"
    />
  </div>
</template>

<style scoped>
/* The classes are on IconButton's inner button, which scoped styles don't reach. */
.direction-pad {
  display: grid;
  grid-template-areas:
    '. up .'
    'left . right'
    '. down .'
    'z-down . z-up';
  grid-template-columns: repeat(3, var(--control-height-md));
  gap: var(--space-1);
  justify-content: center;
}

.direction-pad :deep(.direction-pad__up) {
  grid-area: up;
}

.direction-pad :deep(.direction-pad__left) {
  grid-area: left;
}

.direction-pad :deep(.direction-pad__right) {
  grid-area: right;
}

.direction-pad :deep(.direction-pad__down) {
  grid-area: down;
}

.direction-pad :deep(.direction-pad__z-down) {
  grid-area: z-down;
  margin-top: var(--space-3);
}

.direction-pad :deep(.direction-pad__z-up) {
  grid-area: z-up;
  margin-top: var(--space-3);
}
</style>
