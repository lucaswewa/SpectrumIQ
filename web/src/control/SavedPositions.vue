<script setup lang="ts">
// The saved positions: save where the stage is, rename, go back, or forget. Going to one moves
// the stage, then the focus. Kept in this browser for now.
import remove from '@material-symbols/svg-400/outlined/delete.svg?raw'
import save from '@material-symbols/svg-400/outlined/bookmark_add.svg?raw'
import { ref } from 'vue'

import { useFocusStore } from '@/stores/focus'
import { useStageStore } from '@/stores/stage'
import AppButton from '@/ui/AppButton.vue'
import IconButton from '@/ui/IconButton.vue'
import TextField from '@/ui/TextField.vue'

import { useSavedPositions, type SavedPosition } from './positions'

const stage = useStageStore()
const focus = useFocusStore()
const saved = useSavedPositions()
const going = ref<number>()

async function go(position: SavedPosition) {
  going.value = position.id
  try {
    const arrived = await stage.moveTo({ x: position.x, y: position.y, z: position.z })
    if (arrived && position.focus !== undefined) await focus.moveTo(position.focus)
  } finally {
    going.value = undefined
  }
}
</script>

<template>
  <div class="saved-positions">
    <AppButton
      size="sm"
      :icon="save"
      :disabled="!stage.position"
      @click="saved.add(stage.position!, focus.position)"
    >
      Save this position
    </AppButton>
    <p v-if="saved.positions.length === 0" class="saved-positions__empty">None saved yet.</p>
    <ul class="saved-positions__list">
      <li v-for="position in saved.positions" :key="position.id" class="saved-positions__item">
        <TextField v-model="position.name" size="sm" :aria-label="`Name of ${position.name}`" />
        <AppButton
          size="sm"
          :disabled="going !== undefined || !stage.fitted"
          :title="`x ${position.x}, y ${position.y}, z ${position.z} µm`"
          @click="go(position)"
        >
          {{ going === position.id ? 'Going…' : 'Go' }}
        </AppButton>
        <IconButton
          :icon="remove"
          :label="`Forget ${position.name}`"
          size="sm"
          variant="ghost"
          @click="saved.remove(position.id)"
        />
      </li>
    </ul>
  </div>
</template>

<style scoped>
.saved-positions {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.saved-positions__empty {
  margin: 0;
  color: var(--color-text-muted);
  font-size: var(--font-size-sm);
}

.saved-positions__list {
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
  margin: 0;
  padding: 0;
  list-style: none;
}

.saved-positions__item {
  display: grid;
  grid-template-columns: 1fr auto auto;
  gap: var(--space-1);
  align-items: center;
}
</style>
