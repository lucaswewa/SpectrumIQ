import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

/** Where the saved positions are kept, in this browser, until the server keeps them (P6.5). */
export const POSITIONS_KEY = 'spectrumiq.positions'

/** A saved position, in µm: the stage's, and the focus's if it was known. */
export interface SavedPosition {
  id: number
  name: string
  x: number
  y: number
  z: number
  focus?: number
}

function valid(entry: unknown): entry is SavedPosition {
  const position = entry as SavedPosition
  return (
    Number.isInteger(position?.id) &&
    typeof position.name === 'string' &&
    [position.x, position.y, position.z].every(Number.isFinite) &&
    (position.focus === undefined || Number.isFinite(position.focus))
  )
}

/** The saved positions, leaving out any that make no sense. */
function read(): SavedPosition[] {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(POSITIONS_KEY) ?? '[]')
    return Array.isArray(parsed) ? parsed.filter(valid) : []
  } catch {
    return []
  }
}

/** The stage positions saved in this browser, to go back to: named, and renamed as wanted. */
export const useSavedPositions = defineStore('positions', () => {
  const positions = ref<SavedPosition[]>(read())
  watch(
    positions,
    () => {
      try {
        window.localStorage.setItem(POSITIONS_KEY, JSON.stringify(positions.value))
      } catch {
        // Without storage, they last until the page reloads.
      }
    },
    { deep: true },
  )

  function add(at: { x: number; y: number; z: number }, focus?: number) {
    const id = Math.max(0, ...positions.value.map((position) => position.id)) + 1
    positions.value.push({ id, name: `Position ${id}`, x: at.x, y: at.y, z: at.z, focus })
  }

  function remove(id: number) {
    positions.value = positions.value.filter((position) => position.id !== id)
  }

  return { positions, add, remove }
})
