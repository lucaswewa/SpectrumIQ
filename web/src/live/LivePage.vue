<script setup lang="ts">
// The Live page (ADR-0038): the overview fills the view, with the ROI on it, which a drag
// chooses; the ROI stream at 1:1 or 2:1 beside it or in its corner; and the camera, display and
// focus controls in the pane. Both streams run through the stream budget. The stage, the focus
// and the peripherals are driven from here too (ADR-0039): the pane, the keys, a double-click on
// the overview, the wheel on the images, and STOP, always in the toolbar.
import pip from '@material-symbols/svg-400/outlined/picture_in_picture.svg?raw'
import split from '@material-symbols/svg-400/outlined/vertical_split.svg?raw'
import { computed, ref } from 'vue'

import { useClickToMove } from '@/control/clickToMove'
import DirectionPad from '@/control/DirectionPad.vue'
import { useNavigationPreferences } from '@/control/navigation'
import NavigationSection from '@/control/NavigationSection.vue'
import PeripheralsPanel from '@/control/PeripheralsPanel.vue'
import PositionSection from '@/control/PositionSection.vue'
import SavedPositions from '@/control/SavedPositions.vue'
import StopButton from '@/control/StopButton.vue'
import { useJog } from '@/control/useJog'
import { wheelFocus } from '@/control/wheel'
import { useCameraStore } from '@/stores/camera'
import { useDisplayStore } from '@/stores/display'
import AccordionSection from '@/ui/AccordionSection.vue'
import AppAccordion from '@/ui/AppAccordion.vue'
import AppButton from '@/ui/AppButton.vue'
import ControlPane from '@/ui/ControlPane.vue'
import IconButton from '@/ui/IconButton.vue'

import CameraControls from './CameraControls.vue'
import type { Size } from './contain'
import DisplayControls from './DisplayControls.vue'
import FocusSparkline from './FocusSparkline.vue'
import type { Rect } from './geometry'
import LiveImage from './LiveImage.vue'
import MiniStream from './MiniStream.vue'
import { useLiveImagePreferences } from './preferences'
import RoiSelector from './RoiSelector.vue'

const camera = useCameraStore()
const display = useDisplayStore()
const preferences = useLiveImagePreferences()
const navigation = useNavigationPreferences()
const motion = useJog()
const moveTo = useClickToMove()
const onWheel = wheelFocus(
  (dz) => motion.focus.add({ x: 0, y: 0, z: dz }),
  () => navigation.steps.focus,
)

const sections = ref(['camera', 'stage', 'display', 'focus'])
const overview = ref<Size>()
const readout = computed(() =>
  camera.region && camera.binning ? { region: camera.region, binning: camera.binning } : undefined,
)
const zoom = computed(() => (display.roiRegion?.zoom === 'half' ? 2 : 1))

function choose(region: Rect) {
  void display.setRoiRegion({ region, zoom: display.roiRegion?.zoom ?? 'native' })
}

function setZoom(next: 1 | 2) {
  const region = display.roiRegion?.region ?? null
  void display.setRoiRegion({ region, zoom: next === 2 ? 'half' : 'native' })
}
</script>

<template>
  <div class="live-page">
    <ControlPane need="operate" width="wide">
      <AppAccordion v-model="sections">
        <AccordionSection value="camera" title="Camera"><CameraControls /></AccordionSection>
        <AccordionSection value="stage" title="Stage and focus">
          <DirectionPad />
          <PositionSection />
        </AccordionSection>
        <AccordionSection value="positions" title="Saved positions">
          <SavedPositions />
        </AccordionSection>
        <AccordionSection value="peripherals" title="Peripherals">
          <PeripheralsPanel />
        </AccordionSection>
        <AccordionSection value="display" title="Display"><DisplayControls /></AccordionSection>
        <AccordionSection value="focus" title="Focus">
          <FocusSparkline :value="display.focusMeasure" />
        </AccordionSection>
        <AccordionSection value="navigation" title="Steps"><NavigationSection /></AccordionSection>
      </AppAccordion>
    </ControlPane>
    <section class="live-page__view" aria-labelledby="live-page-heading">
      <h1 id="live-page-heading" class="visually-hidden">Live</h1>
      <div class="live-page__toolbar">
        <StopButton class="live-page__stop" />
        <div role="group" aria-label="ROI zoom" class="live-page__group">
          <AppButton
            v-for="option in [1, 2] as const"
            :key="option"
            size="sm"
            :variant="zoom === option ? 'primary' : 'default'"
            :aria-pressed="zoom === option"
            @click="setZoom(option)"
          >
            {{ option === 1 ? '1:1' : '2:1' }}
          </AppButton>
        </div>
        <IconButton
          :icon="preferences.roiLayout === 'split' ? pip : split"
          :label="preferences.roiLayout === 'split' ? 'ROI in the corner' : 'ROI beside'"
          size="sm"
          @click="preferences.roiLayout = preferences.roiLayout === 'split' ? 'pip' : 'split'"
        />
      </div>
      <div
        class="live-page__images"
        :class="`live-page__images--${preferences.roiLayout}`"
        @wheel="onWheel"
      >
        <RoiSelector
          :readout="readout"
          :image="overview"
          :region="display.roiRegion?.region"
          :zoom="zoom"
          @select="choose"
        >
          <LiveImage
            stream="overview"
            @frame-size="(size) => (overview = size)"
            @double-click="moveTo"
          />
        </RoiSelector>
        <LiveImage v-if="preferences.roiLayout === 'split'" stream="roi" class="live-page__roi" />
        <MiniStream v-else class="live-page__pip" />
      </div>
    </section>
  </div>
</template>

<style scoped>
.live-page {
  display: flex;
  flex: 1;
  min-width: 0;
  min-height: 0;
}

.live-page__view {
  display: flex;
  flex: 1;
  flex-direction: column;
  min-width: 0;
}

.live-page__toolbar {
  display: flex;
  gap: var(--space-3);
  align-items: center;
  justify-content: flex-end;
  padding: var(--space-2) var(--space-3);
  border-bottom: var(--border-width) solid var(--color-border);
}

.live-page__stop {
  margin-right: auto;
}

.live-page__group {
  display: flex;
  gap: var(--space-1);
}

.live-page__images {
  position: relative;
  display: flex;
  flex: 1;
  gap: var(--border-width);
  min-height: 0;
  background: var(--color-border);
}

.live-page__roi {
  flex: 1;
}

.live-page__pip {
  position: absolute;
  right: var(--space-4);
  bottom: var(--space-4);
  box-shadow: var(--shadow-lg);
}
</style>
