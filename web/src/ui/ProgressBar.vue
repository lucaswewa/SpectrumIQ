<script setup lang="ts">
// How far a task has got: `value` out of `max`, or, without a value, a bar
// that shows the task is running for an unknown time. `label` names it for
// assistive technology.
import { computed } from 'vue'

const props = withDefaults(defineProps<{ value?: number | null; max?: number; label: string }>(), {
  value: null,
  max: 100,
})

/** The value within 0–max, or null while indeterminate. */
const current = computed(() =>
  props.value === null ? null : Math.min(Math.max(props.value, 0), props.max),
)
</script>

<template>
  <div
    class="progress-bar"
    :class="{ 'progress-bar--indeterminate': current === null }"
    role="progressbar"
    :aria-label="label"
    aria-valuemin="0"
    :aria-valuemax="max"
    :aria-valuenow="current ?? undefined"
  >
    <div
      class="progress-bar__fill"
      :style="current === null ? undefined : { width: `${(current / max) * 100}%` }"
    />
  </div>
</template>

<style scoped>
.progress-bar {
  height: 6px;
  overflow: hidden;
  border-radius: var(--radius-full);
  background: var(--color-border);
}

.progress-bar__fill {
  height: 100%;
  border-radius: inherit;
  background: var(--color-accent);
  transition: width var(--duration-normal) var(--easing-standard);
}

.progress-bar--indeterminate .progress-bar__fill {
  width: 40%;
  animation: progress-bar-slide 1.4s var(--easing-standard) infinite;
}

@keyframes progress-bar-slide {
  from {
    transform: translateX(-100%);
  }

  to {
    transform: translateX(250%);
  }
}

/* Without motion, a full, faint bar. */
@media (prefers-reduced-motion: reduce) {
  .progress-bar--indeterminate .progress-bar__fill {
    width: 100%;
    opacity: 0.5;
    animation: none;
  }
}
</style>
