<script setup lang="ts">
// A step of the calibration wizard that puts the task's objective and channel in use: the turret
// turned to the objective, if one is fitted, and the channel applied. Next says Skip until both
// are in use. Without a turret, the objective is put in place by hand.
import { computed, watch } from 'vue'

import { useImagingStore } from '@/stores/imaging'
import { usePeripheralsStore } from '@/stores/peripherals'
import AppButton from '@/ui/AppButton.vue'

import type { Channel, Objective } from './tasks'

const props = defineProps<{ objective: Objective; channel?: Channel }>()
const emit = defineEmits<{ waiting: [waiting: boolean] }>()
const peripherals = usePeripheralsStore()
const imaging = useImagingStore()

const turret = peripherals.objectives
const position = computed(
  () => turret.configured?.find((each) => each.id === props.objective.id)?.turret_position,
)
const objectiveInPlace = computed(() => !turret.fitted || turret.current?.id === props.objective.id)
const channelInUse = computed(() => {
  const wanted = props.channel?.id
  return wanted == null || imaging.channel === wanted
})
watch(
  [objectiveInPlace, channelInUse],
  ([objective, channel]) => emit('waiting', !(objective && channel)),
  { immediate: true },
)
</script>

<template>
  <div class="wizard-step">
    <p>
      This calibrates {{ objective.name
      }}<template v-if="channel?.id != null"> in {{ channel.name }}</template
      >.
    </p>
    <p v-if="!turret.fitted">Make sure {{ objective.name }} is in place.</p>
    <p v-else-if="objectiveInPlace">{{ objective.name }} is in place.</p>
    <div v-else class="wizard-step__actions">
      <AppButton
        variant="primary"
        :disabled="position == null || turret.moving"
        @click="peripherals.selectObjective(position!)"
      >
        Turn to {{ objective.name }}
      </AppButton>
    </div>
    <template v-if="channel?.id != null">
      <p v-if="channelInUse">{{ channel.name }} is in use.</p>
      <div v-else class="wizard-step__actions">
        <AppButton variant="primary" @click="imaging.selectChannel(channel.id!)">
          Use {{ channel.name }}
        </AppButton>
      </div>
    </template>
  </div>
</template>
