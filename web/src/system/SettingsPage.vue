<script setup lang="ts">
// The Settings page: this computer's serial ports, each with the device the configuration puts on
// it, and the configured ports it doesn't have (P10.3), for writing `[stage] port` and
// `[focus] port`. The configuration itself is a file: the service reads it at start.
import { onMounted, ref } from 'vue'

import { siq } from '@/api/siq'
import AppButton from '@/ui/AppButton.vue'
import MainView from '@/ui/MainView.vue'
import SectionHeading from '@/ui/SectionHeading.vue'

/** `GET /api/siq/system/ports`. */
interface Ports {
  ports: { name: string; description: string; device: string | null }[]
  missing: { name: string; device: string }[]
  error: string | null
}

const found = ref<Ports>()
const failed = ref<string>()

async function look() {
  failed.value = undefined
  try {
    found.value = (await siq('system/ports')) as Ports
  } catch (error) {
    failed.value = String(error)
  }
}
onMounted(look)
</script>

<template>
  <MainView class="settings-page">
    <h1>Settings</h1>
    <section>
      <SectionHeading>Serial ports</SectionHeading>
      <p class="settings-page__lead">
        The ports this computer has. The stage's and the focus drive's are set in the configuration,
        as <code>[stage] port</code> and <code>[focus] port</code>.
      </p>
      <p v-if="failed || found?.error" role="alert">{{ failed ?? found?.error }}</p>
      <table v-if="found?.ports.length" class="settings-page__ports">
        <thead>
          <tr>
            <th>Port</th>
            <th>What it is</th>
            <th>Configured for</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="port in found.ports" :key="port.name">
            <td>{{ port.name }}</td>
            <td>{{ port.description }}</td>
            <td>{{ port.device ?? '—' }}</td>
          </tr>
        </tbody>
      </table>
      <p v-else-if="found">No serial ports.</p>
      <p v-for="port in found?.missing ?? []" :key="port.name" role="alert">
        The {{ port.device }} is configured on {{ port.name }}, which this computer doesn't have.
      </p>
      <AppButton size="sm" @click="look()">Look again</AppButton>
    </section>
  </MainView>
</template>

<style scoped>
.settings-page h1 {
  margin: 0 0 var(--space-4);
  font-size: var(--font-size-xl);
}

.settings-page__lead {
  margin: 0 0 var(--space-3);
  color: var(--color-text-muted);
}

.settings-page__ports {
  margin: 0 0 var(--space-3);
  border-collapse: collapse;
}

.settings-page__ports th,
.settings-page__ports td {
  padding: var(--space-1) var(--space-4) var(--space-1) 0;
  text-align: left;
}

.settings-page__ports th {
  color: var(--color-text-muted);
  font-weight: normal;
}
</style>
