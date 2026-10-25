<script setup lang="ts">
// The design tokens in both themes, side by side. For development only:
// the route exists only in development builds (src/router/index.ts).
import { onMounted, ref, useTemplateRef } from 'vue'

import { useThemeStore, type Theme, type ThemePreference } from '@/theme/store'
import { TOKEN_GROUPS } from '@/theme/tokens'

const theme = useThemeStore()
const choices: { value: ThemePreference; label: string }[] = [
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
  { value: 'system', label: 'Follow system' },
]
const themes: Theme[] = ['light', 'dark']

const panels = useTemplateRef<HTMLElement[]>('panels')
const values = ref<Record<string, Record<string, string>>>({})

onMounted(() => {
  for (const panel of panels.value ?? []) {
    const style = getComputedStyle(panel)
    const name = panel.dataset.theme ?? ''
    values.value[name] = Object.fromEntries(
      Object.values(TOKEN_GROUPS)
        .flat()
        .map((token) => [token, style.getPropertyValue(token).trim()]),
    )
  }
})

const swatchGroups = ['palette', 'color'] as const
</script>

<template>
  <main class="tokens">
    <header class="tokens__header">
      <h1>Design tokens</h1>
      <p class="tokens__note">
        For development only. Each panel forces its theme; the switcher sets the page's.
      </p>
      <fieldset class="tokens__switcher">
        <legend>Theme</legend>
        <label v-for="choice in choices" :key="choice.value">
          <input
            type="radio"
            name="theme"
            :value="choice.value"
            :checked="theme.preference === choice.value"
            @change="theme.setPreference(choice.value)"
          />
          {{ choice.label }}
        </label>
        <span class="tokens__note">Applied: {{ theme.theme }}</span>
      </fieldset>
    </header>

    <div class="tokens__panels">
      <section
        v-for="name in themes"
        ref="panels"
        :key="name"
        :data-theme="name"
        class="tokens__panel"
      >
        <h2>{{ name === 'light' ? 'Light' : 'Dark' }}</h2>

        <template v-for="group in swatchGroups" :key="group">
          <h3>{{ group === 'palette' ? 'Palette' : 'Colours' }}</h3>
          <ul class="tokens__list">
            <li v-for="token in TOKEN_GROUPS[group]" :key="token" class="tokens__row">
              <span class="tokens__swatch" :style="{ background: `var(${token})` }" />
              <code>{{ token }}</code>
              <span class="tokens__value">{{ values[name]?.[token] }}</span>
            </li>
          </ul>
        </template>

        <h3>Typography</h3>
        <p
          v-for="token in TOKEN_GROUPS.typography.filter((t) => t.startsWith('--font-size'))"
          :key="token"
          :style="{ fontSize: `var(${token})` }"
          class="tokens__sample"
        >
          Slide scan {{ values[name]?.[token] }}
        </p>
        <p class="tokens__sample" :style="{ fontWeight: 'var(--font-weight-light)' }">
          Light 300 · <strong>Semibold 600</strong> ·
          <code :style="{ fontFamily: 'var(--font-family-mono)' }">mono 0123</code>
        </p>

        <h3>Spacing</h3>
        <ul class="tokens__list">
          <li v-for="token in TOKEN_GROUPS.spacing" :key="token" class="tokens__row">
            <span class="tokens__bar" :style="{ width: `var(${token})` }" />
            <code>{{ token }}</code>
            <span class="tokens__value">{{ values[name]?.[token] }}</span>
          </li>
        </ul>

        <h3>Radii, controls and elevation</h3>
        <div class="tokens__boxes">
          <span
            v-for="token in TOKEN_GROUPS.radius"
            :key="token"
            class="tokens__box"
            :style="{ borderRadius: `var(${token})` }"
            >{{ token.replace('--radius-', '') }}</span
          >
        </div>
        <div class="tokens__boxes">
          <span class="tokens__control" :style="{ height: 'var(--control-height-sm)' }">sm</span>
          <span class="tokens__control" :style="{ height: 'var(--control-height-md)' }">md</span>
          <span class="tokens__control tokens__control--accent">Accent</span>
          <button type="button" class="tokens__control">Focus me</button>
        </div>
        <div class="tokens__boxes">
          <span
            v-for="token in TOKEN_GROUPS.elevation"
            :key="token"
            class="tokens__box tokens__box--raised"
            :style="{ boxShadow: `var(${token})` }"
            >{{ token.replace('--shadow-', '') }}</span
          >
        </div>

        <h3>Layout, layers and motion</h3>
        <ul class="tokens__list">
          <li
            v-for="token in [
              ...TOKEN_GROUPS.layout,
              ...TOKEN_GROUPS.annotation,
              ...TOKEN_GROUPS.layer,
              ...TOKEN_GROUPS.motion,
            ]"
            :key="token"
            class="tokens__row"
          >
            <code>{{ token }}</code>
            <span class="tokens__value">{{ values[name]?.[token] }}</span>
          </li>
        </ul>
      </section>
    </div>
  </main>
</template>

<style scoped>
.tokens {
  padding: var(--space-6);
}

.tokens__note {
  color: var(--color-text-muted);
}

.tokens__switcher {
  display: flex;
  gap: var(--space-4);
  align-items: center;
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-sm);
}

.tokens__panels {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: var(--space-6);
  margin-top: var(--space-6);
}

.tokens__panel {
  padding: var(--space-6);
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-bg);
  color: var(--color-text);
}

.tokens__list {
  margin: 0 0 var(--space-6);
  padding: 0;
  list-style: none;
}

.tokens__row {
  display: flex;
  gap: var(--space-3);
  align-items: center;
  min-height: var(--space-8);
}

.tokens__value {
  margin-left: auto;
  color: var(--color-text-muted);
  font-family: var(--font-family-mono);
  font-size: var(--font-size-xs);
}

.tokens__swatch {
  flex: none;
  width: var(--space-8);
  height: var(--space-6);
  border: var(--border-width) solid var(--color-border-strong);
  border-radius: var(--radius-sm);
}

.tokens__bar {
  flex: none;
  height: var(--space-3);
  background: var(--color-accent);
}

.tokens__sample {
  margin: 0 0 var(--space-2);
}

.tokens__boxes {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-4);
  align-items: center;
  margin-bottom: var(--space-6);
}

.tokens__box {
  display: grid;
  place-items: center;
  width: 72px;
  height: 48px;
  border: var(--border-width) solid var(--color-control-border);
  font-size: var(--font-size-xs);
}

.tokens__box--raised {
  border: none;
  border-radius: var(--radius-md);
  background: var(--color-surface-raised);
}

.tokens__control {
  display: inline-grid;
  place-items: center;
  min-width: 72px;
  height: var(--control-height-md);
  padding: 0 var(--control-padding-x);
  border: var(--border-width) solid var(--color-control-border);
  border-radius: var(--radius-sm);
  background: var(--color-control-bg);
  color: var(--color-control-text);
  font: inherit;
}

.tokens__control--accent {
  border-color: var(--color-accent);
  background: var(--color-accent);
  color: var(--color-on-accent);
}
</style>
