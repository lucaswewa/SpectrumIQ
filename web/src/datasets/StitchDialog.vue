<script setup lang="ts">
// The stitch options dialog (ADR-0064): a dataset stitched into an image of a name, the stage's
// uncertainty its search reaches by, the feather, and whole pixels or not; Stitch starts the job,
// told in a toast. The rest is the server's defaults.
import { ref, watch } from 'vue'

import { useJobsStore } from '@/stores/jobs'
import type { DatasetEntry } from '@/stores/datasets'
import AppButton from '@/ui/AppButton.vue'
import AppCheckbox from '@/ui/AppCheckbox.vue'
import AppDialog from '@/ui/AppDialog.vue'
import FormField from '@/ui/FormField.vue'
import NumberField from '@/ui/NumberField.vue'
import { useToast } from '@/ui/overlays'
import TextField from '@/ui/TextField.vue'

const props = defineProps<{ entry: DatasetEntry }>()
const open = defineModel<boolean>('open', { default: false })

const jobs = useJobsStore()
const toast = useToast()
const image = ref('stitched')
const stage = ref(5)
const feather = ref(128)
const integer = ref(false)
watch(open, (now) => now && (image.value = 'stitched'))

async function stitch() {
  const options = {
    pairs: { stage_sigma_um: stage.value },
    solve: { stage_sigma_um: stage.value },
    composite: { feather_px: feather.value, integer: integer.value },
  }
  const job = await jobs.stitch(props.entry.id, image.value.trim(), options)
  if (job === undefined) return
  toast.success(`${props.entry.name} is being stitched into ${image.value}: job ${job}.`)
  open.value = false
}
</script>

<template>
  <AppDialog v-model:open="open" :title="`Stitch ${entry.name}`" size="sm">
    <div class="stitch-dialog">
      <FormField label="Image">
        <TextField v-model="image" />
      </FormField>
      <FormField label="The stage's uncertainty, µm" help="How far the search reaches">
        <NumberField v-model="stage" :min="0.5" :max="100" />
      </FormField>
      <FormField label="Feather, pixels" help="How far in a tile fades in">
        <NumberField v-model="feather" :min="1" :max="1024" :step="1" />
      </FormField>
      <AppCheckbox v-model="integer" label="Whole pixels: no resampling" />
    </div>
    <template #footer>
      <AppButton @click="open = false">Cancel</AppButton>
      <AppButton variant="primary" :disabled="!/^[\w-]+$/.test(image.trim())" @click="stitch">
        Stitch
      </AppButton>
    </template>
  </AppDialog>
</template>

<style scoped>
.stitch-dialog {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
</style>
