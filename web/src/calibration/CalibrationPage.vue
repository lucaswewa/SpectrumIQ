<script setup lang="ts">
// The Calibration page (ADR-0051): each objective, with its turret position, its parfocal offset,
// its stage–camera calibration in force and whether scans may run with it; each objective and
// channel's dark and flat; and the wizard, for any of them or for everything missing. Its buttons
// need control of the instrument (ADR-0068).
import { computed } from 'vue'

import { useGate } from '@/auth/gate'
import { useCalibrationStore, type ObjectiveStatus } from '@/stores/calibration'
import { usePeripheralsStore } from '@/stores/peripherals'
import AppButton from '@/ui/AppButton.vue'
import GatedFields from '@/ui/GatedFields.vue'
import MainView from '@/ui/MainView.vue'
import SectionHeading from '@/ui/SectionHeading.vue'

import { scanGate } from './gating'
import { channelsOf, shadingTask, stageCameraTask, summary } from './tasks'
import { useCalibrationWizard } from './wizard'

const calibration = useCalibrationStore()
const peripherals = usePeripheralsStore()
const wizard = useCalibrationWizard()
const refused = useGate(() => 'operate')

const date = (created: string) => new Date(created).toLocaleDateString()

const objectives = computed(() =>
  (calibration.status ?? []).map((entry) => {
    const channels = channelsOf(entry)
    const ids = channels.map((shading) => shading.channel ?? null)
    return {
      entry,
      config: peripherals.objectives.configured?.find((each) => each.id === entry.objective),
      channels,
      gate: scanGate(calibration.status, entry.objective, ids),
    }
  }),
)
const notes = computed(() =>
  objectives.value.flatMap(({ gate }) => [...gate.blockers, ...gate.warnings]),
)
const objectiveOf = (entry: ObjectiveStatus) => ({ id: entry.objective, name: entry.name })
const value = (facts: [string, string][]) => facts.map(([, fact]) => fact).join('; ')
</script>

<template>
  <MainView class="calibration-page">
    <header class="calibration-page__header">
      <h1>Calibration</h1>
      <AppButton variant="primary" :disabled="refused !== undefined" @click="wizard.openNeeded()">
        Calibrate what's missing
      </AppButton>
    </header>
    <p v-if="!calibration.status" class="calibration-page__empty">
      The calibrations appear here once the server answers.
    </p>
    <GatedFields v-else need="operate">
      <section>
        <SectionHeading>Objectives</SectionHeading>
        <table class="calibration-page__table">
          <thead>
            <tr>
              <th>Objective</th>
              <th>Turret</th>
              <th>Parfocal offset</th>
              <th>Stage–camera</th>
              <th>Scans</th>
              <th />
            </tr>
          </thead>
          <tbody>
            <tr v-for="{ entry, config, gate } in objectives" :key="entry.objective">
              <th scope="row">{{ entry.name }}</th>
              <td>{{ config?.turret_position ?? '—' }}</td>
              <td>{{ config ? `${config.parfocal_offset} µm` : '—' }}</td>
              <td v-if="entry.calibration">
                {{ value(summary('stage_camera', entry.calibration).slice(0, 2)) }}, on
                {{ date(entry.calibration.created) }}
              </td>
              <td v-else class="calibration-page__missing">Not calibrated</td>
              <td :class="{ 'calibration-page__missing': !gate.ready }">
                {{
                  gate.ready ? (gate.warnings.length ? 'Ready, with warnings' : 'Ready') : 'Blocked'
                }}
              </td>
              <td>
                <AppButton size="sm" @click="wizard.start([stageCameraTask(objectiveOf(entry))])">
                  {{ entry.calibration ? 'Calibrate again' : 'Calibrate' }}
                </AppButton>
              </td>
            </tr>
          </tbody>
        </table>
        <ul v-if="notes.length" class="calibration-page__notes">
          <li v-for="note in notes" :key="note">{{ note }}</li>
        </ul>
      </section>
      <section>
        <SectionHeading>Dark and flat</SectionHeading>
        <table class="calibration-page__table">
          <thead>
            <tr>
              <th>Objective</th>
              <th>Channel</th>
              <th>Dark</th>
              <th>Flat</th>
              <th />
            </tr>
          </thead>
          <tbody>
            <template v-for="{ entry, channels } in objectives" :key="entry.objective">
              <tr v-for="shading in channels" :key="shading.channel ?? 'none'">
                <th scope="row">{{ entry.name }}</th>
                <td>{{ shading.name }}</td>
                <td v-if="shading.dark">{{ value(summary('dark', shading.dark)) }}</td>
                <td v-else class="calibration-page__missing">None</td>
                <td v-if="shading.flat">
                  {{ value(summary('flat', shading.flat)) }}, by {{ shading.flat.method }}
                </td>
                <td v-else class="calibration-page__missing">None</td>
                <td>
                  <AppButton
                    size="sm"
                    @click="
                      wizard.start([
                        shadingTask(objectiveOf(entry), {
                          id: shading.channel ?? null,
                          name: shading.name,
                        }),
                      ])
                    "
                  >
                    {{ shading.flat ? 'Take again' : 'Take' }}
                  </AppButton>
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </section>
    </GatedFields>
  </MainView>
</template>

<style scoped>
.calibration-page__header {
  display: flex;
  gap: var(--space-4);
  align-items: center;
  justify-content: space-between;
}

.calibration-page__header h1 {
  margin: 0;
}

.calibration-page section {
  margin-top: var(--space-6);
}

.calibration-page__table {
  width: 100%;
  border-collapse: collapse;
  font-variant-numeric: tabular-nums;
}

.calibration-page__table th,
.calibration-page__table td {
  padding: var(--space-2) var(--space-3);
  border-bottom: var(--border-width) solid var(--color-border);
  text-align: left;
}

.calibration-page__table thead th {
  color: var(--color-text-muted);
  font-weight: normal;
}

.calibration-page__missing {
  color: var(--color-warning);
}

.calibration-page__notes {
  margin: var(--space-3) 0 0;
  color: var(--color-text-muted);
}

.calibration-page__empty {
  color: var(--color-text-muted);
}
</style>
