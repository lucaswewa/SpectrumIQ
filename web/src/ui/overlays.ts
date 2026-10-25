import { inject, type InjectionKey, type VNodeChild } from 'vue'

/** A question for `useConfirm()`. */
export interface ConfirmOptions {
  title: string
  message?: string
  /** More content below the message, as a render function: `() => h('ul', …)`. */
  content?: () => VNodeChild
  /** The confirming button's label: "Confirm" unless given. */
  confirmLabel?: string
  /** The other button's label: "Cancel" unless given. */
  cancelLabel?: string
  /** Shows the confirming button as destructive. */
  danger?: boolean
}

/** Asks a question in a dialog: true if confirmed, false if cancelled or dismissed. */
export type Confirm = (options: ConfirmOptions) => Promise<boolean>

export type ToastKind = 'success' | 'info' | 'error'

export interface ToastOptions {
  /** Shown folded under the message: text, or an error body (see ErrorDetails). */
  details?: unknown
  /** How long it stays, in ms. Success and info stay 5 s; errors stay until closed. */
  duration?: number
}

/** Shows a short notification in the corner of the window. */
export type Toaster = Record<ToastKind, (message: string, options?: ToastOptions) => void>

export const confirmKey: InjectionKey<Confirm> = Symbol('confirm')
export const toastKey: InjectionKey<Toaster> = Symbol('toast')

/** Asks the user to confirm something. Needs an OverlayProvider above the component. */
export function useConfirm(): Confirm {
  return injectOrThrow(confirmKey, 'useConfirm')
}

/** Shows notifications: `toast.success('Saved')`. Needs an OverlayProvider above the component. */
export function useToast(): Toaster {
  return injectOrThrow(toastKey, 'useToast')
}

function injectOrThrow<T>(key: InjectionKey<T>, name: string): T {
  const value = inject(key, null)
  if (value === null) throw new Error(`${name}() needs an OverlayProvider above this component.`)
  return value
}
