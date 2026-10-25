// @ts-check
import skipFormatting from '@vue/eslint-config-prettier/skip-formatting'
import { defineConfigWithVueTs, vueTsConfigs } from '@vue/eslint-config-typescript'
import pluginVue from 'eslint-plugin-vue'
import { globalIgnores } from 'eslint/config'

const hashHistoryOnly = {
  name: 'vue-router',
  importNames: ['createWebHistory'],
  message: 'Routes live in the hash: use createWebHashHistory (ADR-0008).',
}

const tauriOnlyInHost = {
  group: ['@tauri-apps/*'],
  message: 'Only src/host uses Tauri: go through useHost() (ADR-0040).',
}

// Formatting is Prettier's job (`npm run format:check`), so the ESLint
// rules that overlap with it are turned off.
export default defineConfigWithVueTs(
  { name: 'app/files-to-lint', files: ['**/*.{ts,mts,vue}'] },
  globalIgnores([
    'dist/**',
    'coverage/**',
    'playwright-report/**',
    'test-results/**',
    // Generated from the server's OpenAPI document (architecture §11).
    'src/api/generated/**',
  ]),
  pluginVue.configs['flat/recommended'],
  vueTsConfigs.recommended,
  {
    // The rules that keep the app ready for the Tauri desktop shell (ADR-0008).
    // Only the host adapters may use the platform directly.
    name: 'app/tauri-ready',
    files: ['src/**/*.{ts,vue}'],
    ignores: ['src/host/**'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          paths: [
            hashHistoryOnly,
            {
              name: 'reka-ui',
              message: 'Use the components in src/ui, which wrap Reka UI (ADR-0008).',
            },
          ],
          patterns: [tauriOnlyInHost],
        },
      ],
      'no-restricted-globals': [
        'error',
        {
          name: 'EventSource',
          message: 'Read server-sent events with fetch, so headers can be added (ADR-0008).',
        },
      ],
      'no-restricted-properties': [
        'error',
        {
          object: 'window',
          property: 'open',
          message: 'Open links with useHost().openLink (ADR-0008).',
        },
      ],
    },
  },
  {
    // Only the UI components use Reka UI directly (ADR-0008). Rule options
    // don't merge between blocks, so this repeats the router rule above.
    name: 'app/ui-components',
    files: ['src/ui/**/*.{ts,vue}'],
    rules: {
      'no-restricted-imports': ['error', { paths: [hashHistoryOnly], patterns: [tauriOnlyInHost] }],
    },
  },
  skipFormatting,
)
