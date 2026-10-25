<script setup lang="ts">
// Every UI component in both themes, side by side, sharing their state. For
// development only: the route exists only in development builds
// (src/router/index.ts).
import add from '@material-symbols/svg-400/outlined/add.svg?raw'
import deleteIcon from '@material-symbols/svg-400/outlined/delete.svg?raw'
import download from '@material-symbols/svg-400/outlined/download.svg?raw'
import moreVert from '@material-symbols/svg-400/outlined/more_vert.svg?raw'
import arrowUpward from '@material-symbols/svg-400/outlined/arrow_upward.svg?raw'
import photoCamera from '@material-symbols/svg-400/outlined/photo_camera.svg?raw'
import refresh from '@material-symbols/svg-400/outlined/refresh.svg?raw'
import { computed, h, ref } from 'vue'

import AccordionSection from '@/ui/AccordionSection.vue'
import AppAccordion from '@/ui/AppAccordion.vue'
import AppButton from '@/ui/AppButton.vue'
import AppCard from '@/ui/AppCard.vue'
import AppCheckbox from '@/ui/AppCheckbox.vue'
import AppDialog from '@/ui/AppDialog.vue'
import AppMenu, { type MenuItems } from '@/ui/AppMenu.vue'
import AppPagination from '@/ui/AppPagination.vue'
import AppSlider from '@/ui/AppSlider.vue'
import AppSelect from '@/ui/AppSelect.vue'
import AppSpinner from '@/ui/AppSpinner.vue'
import AppToggle from '@/ui/AppToggle.vue'
import AppTooltip from '@/ui/AppTooltip.vue'
import ButtonMenu from '@/ui/ButtonMenu.vue'
import ErrorDetails from '@/ui/ErrorDetails.vue'
import FormField from '@/ui/FormField.vue'
import MultiSelect from '@/ui/MultiSelect.vue'
import IconButton from '@/ui/IconButton.vue'
import NumberField from '@/ui/NumberField.vue'
import { useConfirm, useToast } from '@/ui/overlays'
import ProgressBar from '@/ui/ProgressBar.vue'
import SectionHeading from '@/ui/SectionHeading.vue'
import TextField from '@/ui/TextField.vue'

const themes = ['light', 'dark'] as const
const variants = ['default', 'primary', 'danger', 'ghost'] as const
const sizes = ['md', 'sm'] as const

const name = ref('')
const exposure = ref(5000)
const stepSize = ref(200)
const resolution = ref<string>()
const resolutions = [
  { value: '820x616', label: '820 × 616' },
  { value: '1640x1232', label: '1640 × 1232' },
  { value: '3280x2464', label: '3280 × 2464 (full)' },
  { value: 'video', label: 'Video only', disabled: true },
]
const saveToGallery = ref(true)
const autoExposure = ref(false)
const brightness = ref(120)
const openSections = ref(['move'])
const progress = ref(35)
const lastEvent = ref('none')

const levels = ref(['WARNING', 'ERROR'])
const levelOptions = ['DEBUG', 'INFO', 'WARNING', 'ERROR'].map((level) => ({
  value: level,
  label: level[0] + level.slice(1).toLowerCase(),
}))
const page = ref(4)
const downloads: MenuItems = [
  { label: 'JPEG', run: () => (lastEvent.value = 'download JPEG') },
  { label: 'TIFF', run: () => (lastEvent.value = 'download TIFF') },
  { label: 'With metadata (ZIP)', disabled: true, run: () => {} },
]
const actions: MenuItems = [
  { label: 'Download', icon: download, run: () => (lastEvent.value = 'action Download') },
  'separator',
  {
    label: 'Delete',
    icon: deleteIcon,
    danger: true,
    run: () => (lastEvent.value = 'action Delete'),
  },
]

const errorSamples = {
  detail: { detail: 'No action found with the name "focus".' },
  invalid: {
    detail: [
      {
        type: 'int_parsing',
        loc: ['body', 'x'],
        msg: 'Input should be a valid integer',
        input: 'ten',
      },
      { type: 'missing', loc: ['body', 'path', 1], msg: 'Field required', input: {} },
    ],
  },
  problem: { title: 'GlobalLockBusyError', detail: 'The instrument is busy.', status: 503 },
}

const dialogOpen = ref(false)
const busyOpen = ref(false)
const moveX = ref(0)
const confirm = useConfirm()
const toast = useToast()

async function ask(danger: boolean) {
  const confirmed = await confirm(
    danger
      ? {
          title: 'Delete 3 captures?',
          message: "This can't be undone.",
          content: () =>
            h(
              'ul',
              ['cell-1.jpg', 'cell-2.jpg', 'cell-3.jpg'].map((n) => h('li', n)),
            ),
          confirmLabel: 'Delete',
          danger: true,
        }
      : { title: 'Move home?', message: 'Take the sample out first.', confirmLabel: 'Move home' },
  )
  lastEvent.value = `confirm ${confirmed}`
}

const state = computed(() =>
  JSON.stringify({
    name: name.value,
    exposure: exposure.value,
    stepSize: stepSize.value,
    resolution: resolution.value ?? null,
    saveToGallery: saveToGallery.value,
    autoExposure: autoExposure.value,
    openSections: openSections.value,
    levels: levels.value,
    page: page.value,
  }),
)
</script>

<template>
  <main class="gallery">
    <h1>Components</h1>
    <p class="gallery__note">
      For development only. Each panel forces its theme; both share their state.
    </p>

    <div class="gallery__panels">
      <section v-for="theme in themes" :key="theme" :data-theme="theme" class="gallery__panel">
        <SectionHeading>
          {{ theme === 'light' ? 'Light' : 'Dark' }}
          <template #actions>
            <IconButton :icon="refresh" label="Refresh" variant="ghost" />
          </template>
        </SectionHeading>

        <h3>Buttons</h3>
        <div v-for="size in sizes" :key="size" class="gallery__row">
          <AppButton v-for="variant in variants" :key="variant" :variant="variant" :size="size">
            {{ variant[0]!.toUpperCase() + variant.slice(1) }}
          </AppButton>
          <AppButton :size="size" :icon="photoCamera" variant="primary">Capture</AppButton>
          <AppButton :size="size" disabled>Disabled</AppButton>
          <IconButton :icon="arrowUpward" label="Move up" :size="size" />
          <IconButton :icon="add" label="Add" :size="size" variant="primary" />
        </div>

        <h3>Fields</h3>
        <div class="gallery__fields">
          <FormField label="Sample name" help="Press Enter to submit.">
            <TextField
              v-model="name"
              placeholder="Untitled"
              @submit="lastEvent = `name ${JSON.stringify($event)}`"
            />
          </FormField>
          <FormField label="Exposure (µs)" help="1–100 000, whole numbers. ↑ and ↓ step.">
            <NumberField
              v-model="exposure"
              :min="1"
              :max="100000"
              :step="1"
              @submit="lastEvent = `exposure ${$event}`"
            />
          </FormField>
          <FormField
            label="Step size (µm)"
            :error="stepSize > 500 ? 'Larger than the field of view.' : undefined"
          >
            <NumberField v-model="stepSize" :min="0" :step="0.5" size="sm" />
          </FormField>
          <FormField label="Resolution">
            <AppSelect v-model="resolution" :options="resolutions" placeholder="Choose…" />
          </FormField>
          <FormField label="Disabled">
            <TextField model-value="Read only" disabled />
          </FormField>
        </div>
        <div class="gallery__row">
          <AppCheckbox v-model="saveToGallery" label="Save to gallery" />
          <AppCheckbox :model-value="true" label="Disabled" disabled />
          <AppToggle v-model="autoExposure" label="Auto exposure" />
          <AppToggle :model-value="false" label="Disabled" disabled />
        </div>
        <div class="gallery__row">
          <AppSlider v-model="brightness" label="Brightness" :max="200" />
          <AppSlider :model-value="50" label="Disabled" disabled />
        </div>

        <h3>Accordion</h3>
        <AppAccordion v-model="openSections">
          <AccordionSection value="move" title="Move">
            <AppButton size="sm">Move to centre</AppButton>
          </AccordionSection>
          <AccordionSection value="capture" title="Capture">
            <AppCheckbox v-model="saveToGallery" label="Save to gallery" />
          </AccordionSection>
          <AccordionSection value="disabled" title="Disabled" disabled>Hidden</AccordionSection>
        </AppAccordion>

        <h3>Menus and lists</h3>
        <div class="gallery__row">
          <ButtonMenu label="Download" :icon="download" :items="downloads" />
          <AppMenu :items="actions">
            <IconButton :icon="moreVert" label="More actions" variant="ghost" />
          </AppMenu>
          <FormField label="Levels" class="gallery__levels">
            <MultiSelect v-model="levels" :options="levelOptions" placeholder="All levels" />
          </FormField>
        </div>
        <AppPagination v-model:page="page" :total="250" :per-page="18" />

        <h3>Progress</h3>
        <div class="gallery__progress">
          <ProgressBar :value="progress" label="Scan progress" />
          <input
            v-model.number="progress"
            type="range"
            min="0"
            max="100"
            aria-label="Progress"
            class="gallery__range"
          />
          <ProgressBar label="Autofocusing" />
          <div class="gallery__row">
            <AppSpinner label="Loading" />
            <AppSpinner size="var(--icon-size-sm)" class="gallery__accent" />
            <AppButton variant="primary" disabled>
              <AppSpinner size="var(--icon-size-sm)" /> Scanning…
            </AppButton>
          </div>
        </div>

        <h3>Card</h3>
        <AppCard>
          <SectionHeading :level="3">Capture</SectionHeading>
          <p class="gallery__note">A card holds a block of related content.</p>
        </AppCard>

        <h3>Error details</h3>
        <div class="gallery__progress">
          <AppCard v-for="(sample, key) in errorSamples" :key="key">
            <ErrorDetails :error="sample" />
          </AppCard>
        </div>
      </section>
    </div>

    <section class="gallery__overlays">
      <h2>Overlays</h2>
      <p class="gallery__note">These open in the page's theme.</p>
      <div class="gallery__row">
        <AppButton @click="dialogOpen = true">Dialog</AppButton>
        <AppButton @click="busyOpen = true">Busy dialog</AppButton>
        <AppButton @click="ask(false)">Confirm</AppButton>
        <AppButton variant="danger" @click="ask(true)">Confirm a deletion</AppButton>
        <AppButton @click="toast.success('Saved to the gallery.')">Success toast</AppButton>
        <AppButton @click="toast.info('Calibration takes about a minute.')">Info toast</AppButton>
        <AppButton
          @click="toast.error('The stage could not move.', { details: errorSamples.invalid })"
        >
          Error toast
        </AppButton>
        <AppTooltip text="Shown on hover and on keyboard focus">
          <AppButton variant="ghost">Tooltip</AppButton>
        </AppTooltip>
      </div>
      <AppDialog v-model:open="dialogOpen" title="Move to" description="In steps from home.">
        <FormField label="x"><NumberField v-model="moveX" /></FormField>
        <template #footer>
          <AppButton @click="dialogOpen = false">Cancel</AppButton>
          <AppButton variant="primary" @click="dialogOpen = false">Move</AppButton>
        </template>
      </AppDialog>
      <AppDialog v-model:open="busyOpen" title="Autofocusing" size="sm" :dismissible="false">
        <ProgressBar label="Autofocusing" />
        <template #footer><AppButton @click="busyOpen = false">Cancel</AppButton></template>
      </AppDialog>
    </section>

    <p class="gallery__state"><strong>Last event:</strong> {{ lastEvent }}</p>
    <p class="gallery__state"><strong>State:</strong> {{ state }}</p>
  </main>
</template>

<style scoped>
.gallery {
  flex: 1;
  padding: var(--space-6);
  overflow: auto;
}

.gallery__note {
  color: var(--color-text-muted);
}

.gallery__panels {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-6);
  margin: var(--space-6) 0;
}

.gallery__panel {
  padding: var(--space-6);
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-surface-panel);
  color: var(--color-text);
}

.gallery__row {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-3);
  align-items: center;
  margin-bottom: var(--space-4);
}

.gallery__fields {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: var(--space-4);
  margin-bottom: var(--space-4);
}

.gallery__progress {
  display: grid;
  gap: var(--space-3);
}

.gallery__accent {
  color: var(--color-accent-text);
}

.gallery__levels {
  width: 220px;
}

.gallery__range {
  accent-color: var(--color-accent);
}

.gallery__state {
  font-family: var(--font-family-mono);
  font-size: var(--font-size-sm);
}
</style>
