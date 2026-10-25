import { readdirSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath, URL } from 'node:url'

import vue from '@vitejs/plugin-vue'
import { defineConfig, type Plugin } from 'vite'

import packageJson from './package.json' with { type: 'json' }

// The development server forwards API requests to a SpectrumIQ server, by
// default one running locally:
//   cargo run --release -p spectrumiq-server -- -c config/sim.toml
// SIQ_API_TARGET points it elsewhere, such as http://lab-pc:5000.
const apiTarget = process.env.SIQ_API_TARGET ?? 'http://127.0.0.1:5000'

/**
 * Writes third-party-licences.txt into the build: each npm package the app
 * bundles, with its licence's text, for the release. The
 * packages are those the bundle's modules come from, development ones such
 * as the icons included.
 */
function thirdPartyLicences(): Plugin {
  return {
    name: 'third-party-licences',
    apply: 'build',
    generateBundle() {
      const packages = new Map<string, string>()
      for (const id of this.getModuleIds()) {
        const found = /^(.*[\\/]node_modules[\\/](@[^\\/]+[\\/][^\\/]+|[^\\/]+))[\\/]/.exec(id)
        if (found) packages.set(found[2]!.replace('\\', '/'), found[1]!)
      }
      const sections = [...packages].sort().map(([name, folder]) => {
        const manifest = JSON.parse(readFileSync(path.join(folder, 'package.json'), 'utf8'))
        const file = readdirSync(folder).find((file) => /^(licen[cs]e|copying)/i.test(file))
        const text = file
          ? readFileSync(path.join(folder, file), 'utf8').trim()
          : `(The package has no licence file; its licence is ${manifest.license}.)`
        return `${name} ${manifest.version}: ${manifest.license}\n\n${text}`
      })
      const source = [
        'Third-party licences: npm packages\n\nThe web app bundles these packages, under these licences.',
        ...sections,
      ].join(`\n\n${'-'.repeat(72)}\n`)
      this.emitFile({ type: 'asset', fileName: 'third-party-licences.txt', source: `${source}\n` })
    },
  }
}

export default defineConfig({
  plugins: [vue(), thirdPartyLicences()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version),
  },
  server: {
    proxy: {
      '/api': { target: apiTarget, ws: true },
    },
  },
})
