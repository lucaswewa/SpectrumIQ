import {
  inject,
  onScopeDispose,
  provide,
  shallowRef,
  type InjectionKey,
  type ShallowRef,
} from 'vue'

/** A keyboard shortcut, as a component registers it with `useShortcut`. */
export interface Shortcut {
  /**
   * The key, as `KeyboardEvent.key`, after any of `Ctrl+`, `Alt+`, `Shift+`
   * and `Meta+`, in that order: `a`, `?`, `Shift+ArrowDown`, `Ctrl+s`. A
   * printable character includes Shift already, so `?` is `?`, and `A` is
   * Shift and `a`.
   */
  keys: string
  /** What it does, for the help dialog. */
  description: string
  /** Its heading in the help dialog. */
  group: string
  run: (event: KeyboardEvent) => void
  /** Whether it also works while typing in a field. Off unless set. */
  inFields?: boolean
}

export interface ShortcutRegistry {
  /** Every registered shortcut, in the order they were registered. */
  readonly shortcuts: Readonly<ShallowRef<readonly Shortcut[]>>
  /** Registers a shortcut, and returns a function that removes it. */
  add(shortcut: Shortcut): () => void
}

const registryKey: InjectionKey<ShortcutRegistry> = Symbol('shortcuts')

/** The `keys` that a key press matches. */
export function keysOf(event: KeyboardEvent): string {
  const printable = event.key.length === 1
  const modifiers = [
    event.ctrlKey && 'Ctrl',
    event.altKey && 'Alt',
    event.shiftKey && !printable && 'Shift',
    event.metaKey && 'Meta',
  ]
  return [...modifiers.filter(Boolean), event.key].join('+')
}

/** Whether a key press is typing into a field. */
function inField(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable || ['INPUT', 'SELECT', 'TEXTAREA'].includes(target.tagName))
  )
}

/**
 * Creates the app's shortcut registry and listens for its keys: called once,
 * by the app shell. A key press is left alone if something already handled
 * it, if it is typing into a field (unless the shortcut allows that), or if
 * it happens in a dialog, which owns the keyboard while it is open.
 */
export function provideShortcuts(target: Window = window): ShortcutRegistry {
  const shortcuts = shallowRef<readonly Shortcut[]>([])

  function onKeydown(event: KeyboardEvent) {
    if (event.defaultPrevented || event.isComposing) return
    if (event.target instanceof Element && event.target.closest('[role$="dialog"]')) return
    const keys = keysOf(event)
    const shortcut = shortcuts.value.find((candidate) => candidate.keys === keys)
    if (!shortcut || (!shortcut.inFields && inField(event.target))) return
    event.preventDefault()
    shortcut.run(event)
  }

  target.addEventListener('keydown', onKeydown)
  onScopeDispose(() => target.removeEventListener('keydown', onKeydown))

  const registry: ShortcutRegistry = {
    shortcuts,
    add(shortcut) {
      const taken = shortcuts.value.find((existing) => existing.keys === shortcut.keys)
      if (taken) {
        throw new Error(`${shortcut.keys} is already the shortcut for "${taken.description}".`)
      }
      shortcuts.value = [...shortcuts.value, shortcut]
      return () => {
        shortcuts.value = shortcuts.value.filter((existing) => existing !== shortcut)
      }
    },
  }
  provide(registryKey, registry)
  return registry
}

/** The app's shortcut registry. */
export function useShortcuts(): ShortcutRegistry {
  const registry = inject(registryKey, null)
  if (registry === null) throw new Error('No shortcut registry: the app shell provides one.')
  return registry
}

/**
 * Registers keyboard shortcuts for as long as the calling component is
 * mounted. Two shortcuts can't share keys: registering a taken one throws.
 */
export function useShortcut(...shortcuts: Shortcut[]) {
  const registry = useShortcuts()
  const removers = shortcuts.map((shortcut) => registry.add(shortcut))
  onScopeDispose(() => removers.forEach((remove) => remove()))
}
