<script setup lang="ts">
// The About page: what the app is, its version, where it runs, and its licence. The server's
// version and the instrument's hardware join it once the client connects (P3.2).
import { useHost } from '@/host'
import MainView from '@/ui/MainView.vue'
import SectionHeading from '@/ui/SectionHeading.vue'

const REPOSITORY = 'https://github.com/lucaswewa/SpectrumIQ'

const host = useHost()
const facts = [
  ['App', host.info.name],
  ['App version', host.info.version],
  ['Running in', host.info.kind === 'tauri' ? 'The desktop app' : 'A browser'],
  ['Licence', 'MIT'],
]

function open(event: MouseEvent) {
  event.preventDefault()
  host.openLink(REPOSITORY)
}
</script>

<template>
  <MainView class="about-page">
    <h1>About</h1>
    <p class="about-page__lead">
      Scientific microscope software: live view, stitching, and 3D, for a 100 MP camera.
    </p>
    <section>
      <SectionHeading>This app</SectionHeading>
      <dl class="about-page__facts">
        <template v-for="[label, value] in facts" :key="label">
          <dt>{{ label }}</dt>
          <dd>{{ value }}</dd>
        </template>
      </dl>
    </section>
    <section>
      <SectionHeading>Source and credits</SectionHeading>
      <p>
        The source is at <a :href="REPOSITORY" @click="open">{{ REPOSITORY }}</a
        >. The web app started from the microscope project's web app, by the same author, under the
        MIT licence; the third-party licences are in <code>third-party-licences.txt</code> beside
        the app.
      </p>
    </section>
  </MainView>
</template>

<style scoped>
.about-page h1 {
  margin: 0 0 var(--space-2);
  font-size: var(--font-size-xl);
}

.about-page__lead {
  margin: 0 0 var(--space-4);
  color: var(--color-text-muted);
}

.about-page__facts {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: var(--space-1) var(--space-4);
  margin: 0 0 var(--space-4);
}

.about-page__facts dt {
  color: var(--color-text-muted);
}

.about-page__facts dd {
  margin: 0;
}
</style>
