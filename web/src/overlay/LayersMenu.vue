<script setup lang="ts">
// The Layers menu (ADR-0009, amended): a choice for each overlay layer, ticked while
// it's shown. The choices are kept in this browser.
import layersIcon from '@material-symbols/svg-400/outlined/layers.svg?raw'
import { computed } from 'vue'

import type { MenuItems } from '@/ui/AppMenu.vue'
import ButtonMenu from '@/ui/ButtonMenu.vue'

import { useOverlayLayers } from './layers'
import type { OverlayLayer } from './types'

const props = defineProps<{ layers: OverlayLayer[] }>()
const choices = useOverlayLayers()

const items = computed<MenuItems>(() =>
  props.layers.map((layer) => ({
    label: layer.label,
    checked: choices.shown(layer.id),
    toggle: (on: boolean) => choices.show(layer.id, on),
  })),
)
</script>

<template>
  <ButtonMenu label="Layers" :icon="layersIcon" :items="items" size="sm" />
</template>
