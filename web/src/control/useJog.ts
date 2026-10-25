import { inject, onBeforeUnmount, onMounted, provide, type InjectionKey } from 'vue'

import { useShortcut } from '@/app/shortcuts'
import { useFocusStore } from '@/stores/focus'
import { useStageStore } from '@/stores/stage'

import { JogController, stageWithinTravel, Stepper, withinTravel, type Axes } from './jog'
import { useNavigationPreferences } from './navigation'

/**
 * Directions as the image shows them: ↑ shows what is above, which is −y (ADR-0039). Z is the
 * stage's coarse z; the focus has its own steps.
 */
export const UP: Axes = { x: 0, y: -1, z: 0 }
export const DOWN: Axes = { x: 0, y: 1, z: 0 }
export const LEFT: Axes = { x: -1, y: 0, z: 0 }
export const RIGHT: Axes = { x: 1, y: 0, z: 0 }
export const Z_UP: Axes = { x: 0, y: 0, z: 1 }
export const Z_DOWN: Axes = { x: 0, y: 0, z: -1 }

const KEYS: [keys: string, direction: Axes, description: string][] = [
  ['ArrowUp', UP, 'Move up'],
  ['ArrowDown', DOWN, 'Move down'],
  ['ArrowLeft', LEFT, 'Move left'],
  ['ArrowRight', RIGHT, 'Move right'],
  ['PageUp', Z_UP, 'Coarse Z up'],
  ['PageDown', Z_DOWN, 'Coarse Z down'],
]

/** A page's motion: the stage's jog, the focus's steps, and STOP. */
export interface Motion {
  controller: JogController
  /** The focus's moves, in µm on z: the wheel's and the focus buttons'. */
  focus: Stepper
  /** STOP: forgets every move still to come, and halts the stage and the focus. */
  stop: () => void
}

const motionKey: InjectionKey<Motion> = Symbol('motion')

/**
 * The page's motion, for the calling component and those inside it (`useMotion`). While it is
 * mounted, the arrow keys and PgUp/PgDn jog the stage, and Esc is STOP wherever the keyboard is,
 * dialogs included. Losing the window's focus, hiding the page or leaving it releases every key,
 * since their key-ups would be missed.
 */
export function useJog(): Motion {
  const stage = useStageStore()
  const focus = useFocusStore()
  const preferences = useNavigationPreferences()
  const controller = new JogController({
    step: (by) => {
      const { x, y, z } = stageWithinTravel(by, stage.position, stage.limits)
      return x || y || z ? stage.moveBy({ dx: x, dy: y, dz: z }) : Promise.resolve()
    },
    jog: ({ x, y, z }) => void stage.jog({ vx: x, vy: y, vz: z }),
    stop: () => void stage.halt(),
    steps: preferences.signedSteps,
  })
  const steps = new Stepper(({ z }) => {
    const dz = withinTravel(z, focus.position, focus.limits)
    return dz ? focus.moveBy(dz) : Promise.resolve()
  })
  const motion: Motion = {
    controller,
    focus: steps,
    stop() {
      controller.cancel()
      steps.clear()
      void stage.halt()
      void focus.halt()
    },
  }
  provide(motionKey, motion)

  useShortcut(
    ...KEYS.map(([keys, direction, description]) => ({
      keys,
      description: `${description}; hold to keep moving`,
      group: 'Stage',
      run: (event: KeyboardEvent) => {
        if (!event.repeat) controller.press(keys, direction)
      },
    })),
  )
  const onEscape = (event: KeyboardEvent) => {
    if (event.key === 'Escape') motion.stop()
  }
  const onKeyup = (event: KeyboardEvent) => controller.release(event.key)
  const releaseAll = () => controller.releaseAll()
  onMounted(() => {
    window.addEventListener('keydown', onEscape, { capture: true })
    window.addEventListener('keyup', onKeyup)
    window.addEventListener('blur', releaseAll)
    window.addEventListener('pagehide', releaseAll)
    document.addEventListener('visibilitychange', releaseAll)
  })
  onBeforeUnmount(() => {
    window.removeEventListener('keydown', onEscape, { capture: true })
    window.removeEventListener('keyup', onKeyup)
    window.removeEventListener('blur', releaseAll)
    window.removeEventListener('pagehide', releaseAll)
    document.removeEventListener('visibilitychange', releaseAll)
    controller.releaseAll()
  })
  return motion
}

/** The motion of the page around the calling component (`useJog`). */
export function useMotion(): Motion {
  const motion = inject(motionKey, null)
  if (motion === null) throw new Error('useMotion() needs useJog() in a component around it.')
  return motion
}
