// The connect page (ADR-0040): it probes the local service's health and, once it answers, loads
// the web app the service serves. Until then it says why, and tries again every 2 s. Development
// builds offer to start the simulated instrument.
const { invoke } = window.__TAURI__.core

const RETRY_MS = 2000
const element = (id) => document.getElementById(id)
const [status, detail, actions, retry, start] = [
  'status',
  'detail',
  'actions',
  'retry',
  'start',
].map(element)

const info = await invoke('app_info')
let timer

async function connect() {
  clearTimeout(timer)
  status.textContent = `Connecting to ${info.server ?? 'the service'}…`
  try {
    await invoke('probe')
    location.replace(info.server)
  } catch (error) {
    status.textContent = info.server
      ? `SpectrumIQ isn't answering at ${info.server}.`
      : "The service's address can't be used."
    const why = String(error).replace(/^./, (first) => first.toUpperCase())
    detail.textContent = info.managesService
      ? `${why}. Start the simulated instrument, or start spectrumiq-server yourself.`
      : `${why}. Start the SpectrumIQ service, and this page connects by itself.`
    actions.hidden = false
    start.hidden = !info.managesService || !info.server
    if (info.server) timer = setTimeout(connect, RETRY_MS)
  }
}

retry.addEventListener('click', connect)
start.addEventListener('click', async () => {
  start.disabled = true
  try {
    await invoke('service_start')
    status.textContent = 'Starting the simulated instrument…'
    timer = setTimeout(connect, RETRY_MS)
  } catch (error) {
    detail.textContent = String(error)
  } finally {
    start.disabled = false
  }
})

await connect()
