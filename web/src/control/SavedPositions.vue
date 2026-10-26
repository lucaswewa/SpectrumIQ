<script setup lang="ts">
// The saved positions, kept by the server: save where the stage and focus are, rename (Enter or
// leaving the name), go back, or forget. Going to one moves the stage, then the focus.
import remove from '@material-symbols/svg-400/outlined/delete.svg?raw'
import save from '@material-symbols/svg-400/outlined/bookmark_add.svg?raw'
import { reactive, ref } from 'vue'

import { useFocusStore } from '@/stores/focus'
import { useStageStore } from '@/stores/stage'
import { useWorkflowStore, type Position } from '@/stores/workflow'
import AppButton from '@/ui/AppButton.vue'
import IconButton from '@/ui/IconButton.vue'
import TextField from '@/ui/TextField.vue'

import { nextName } from './positions'

const stage = useStageStore()
const focus = useFocusStore()
const workflow = useWorkflowStore()
const going = ref<string>()
/** Names being edited, by the name saved. */
const drafts = reactive<Record<string, string>>({})

async function go(position: Position) {
  going.value = position.name
  try {
    const z = position.z ?? stage.position?.z ?? 0
    const arrived = await stage.moveTo({ x: position.x, y: position.y, z })
    if (arrived && position.focus !== undefined && position.focus !== null) {
      await focus.moveTo(position.focus)
    }
  } finally {
    going.value = undefined
  }
}

async function rename(name: string) {
  const to = drafts[name]?.trim()
  delete drafts[name]
  if (to && to !== name) await workflow.renamePosition(name, to)
}
</script>

<template>
  <div class="saved-positions">
    <AppButton
      size="sm"
      :icon="save"
      :disabled="!workflow.fitted || !stage.position"
      @click="workflow.savePosition({ name: nextName(workflow.positions ?? []) })"
    >
      Save this position
    </AppButton>
    <p v-if="!workflow.positions?.length" class="saved-positions__empty">None saved yet.</p>
    <ul class="saved-positions__list">
      <li v-for="position in workflow.positions" :key="position.name" class="saved-positions__item">
        <TextField
          :model-value="drafts[position.name] ?? position.name"
          size="sm"
          :aria-label="`Name of ${position.name}`"
          @update:model-value="drafts[position.name] = $event"
          @submit="rename(position.name)"
          @focusout="rename(position.name)"
        />
        <AppButton
          size="sm"
          :disabled="going !== undefined || !stage.fitted"
          :title="`x ${position.x}, y ${position.y}, z ${position.z} µm`"
          @click="go(position)"
        >
          {{ going === position.name ? 'Going…' : 'Go' }}
        </AppButton>
        <IconButton
          :icon="remove"
          :label="`Forget ${position.name}`"
          size="sm"
          variant="ghost"
          @click="workflow.removePosition(position.name)"
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
