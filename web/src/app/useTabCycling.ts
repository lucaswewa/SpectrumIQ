import type { Ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import type { Destination } from './navigation'
import { useShortcut } from './shortcuts'

/**
 * Shift+↓ and Shift+↑ go to the next and previous destination in the rail,
 * wrapping around. Like every shortcut, they're ignored while typing.
 */
export function useTabCycling(destinations: Ref<readonly Destination[]>) {
  const router = useRouter()
  const route = useRoute()

  function go(step: 1 | -1) {
    const list = destinations.value
    if (list.length === 0) return
    const current = list.findIndex((destination) => destination.id === route.name)
    const next =
      current === -1
        ? list[step > 0 ? 0 : list.length - 1]
        : list[(current + step + list.length) % list.length]
    void router.push({ name: next!.id })
  }

  useShortcut(
    { keys: 'Shift+ArrowDown', description: 'Next tab', group: 'Navigation', run: () => go(1) },
    { keys: 'Shift+ArrowUp', description: 'Previous tab', group: 'Navigation', run: () => go(-1) },
  )
}
