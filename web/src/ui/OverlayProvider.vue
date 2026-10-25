<script setup lang="ts">
// Confirmations and toasts for everything inside it, which useConfirm() and
// useToast() find. App.vue puts one around the whole app.
//
// Confirmations queue, one dialog at a time. Toasts stack in the bottom-right
// corner. Errors are announced at once and stay until closed; success and
// info are announced politely and go after 5 s, though hovering or focusing
// them pauses the time. F8 moves the focus to them.
import checkCircle from '@material-symbols/svg-400/outlined/check_circle.svg?raw'
import close from '@material-symbols/svg-400/outlined/close.svg?raw'
import error from '@material-symbols/svg-400/outlined/error.svg?raw'
import info from '@material-symbols/svg-400/outlined/info.svg?raw'
import {
  ToastClose,
  ToastDescription,
  ToastProvider,
  ToastRoot,
  ToastTitle,
  ToastViewport,
} from 'reka-ui'
import { computed, provide, ref } from 'vue'

import AppButton from './AppButton.vue'
import AppDialog from './AppDialog.vue'
import AppIcon from './AppIcon.vue'
import ErrorDetails from './ErrorDetails.vue'
import IconButton from './IconButton.vue'
import {
  confirmKey,
  toastKey,
  type ConfirmOptions,
  type ToastKind,
  type ToastOptions,
} from './overlays'

const questions = ref<(ConfirmOptions & { answer: (confirmed: boolean) => void })[]>([])
const question = computed(() => questions.value[0])

provide(confirmKey, (options) => {
  return new Promise((resolve) => questions.value.push({ ...options, answer: resolve }))
})

function answer(confirmed: boolean) {
  questions.value.shift()?.answer(confirmed)
}

interface Toast extends ToastOptions {
  id: number
  kind: ToastKind
  message: string
}

const toasts = ref<Toast[]>([])
let nextId = 0
const show =
  (kind: ToastKind) =>
  (message: string, options: ToastOptions = {}) => {
    toasts.value.push({ id: nextId++, kind, message, ...options })
  }
provide(toastKey, { success: show('success'), info: show('info'), error: show('error') })

const icons: Record<ToastKind, string> = { success: checkCircle, info, error }

function remove(id: number) {
  toasts.value = toasts.value.filter((toast) => toast.id !== id)
}
</script>

<template>
  <slot />

  <AppDialog
    :open="question !== undefined"
    :title="question?.title ?? ''"
    size="sm"
    alert
    :close-on-outside-click="false"
    @update:open="(open) => open || answer(false)"
  >
    <p v-if="question?.message" class="overlay-confirm__message">{{ question.message }}</p>
    <component :is="question.content" v-if="question?.content" />
    <template #footer>
      <AppButton @click="answer(false)">{{ question?.cancelLabel ?? 'Cancel' }}</AppButton>
      <AppButton :variant="question?.danger ? 'danger' : 'primary'" @click="answer(true)">
        {{ question?.confirmLabel ?? 'Confirm' }}
      </AppButton>
    </template>
  </AppDialog>

  <ToastProvider label="Notification" swipe-direction="right">
    <ToastRoot
      v-for="toast in toasts"
      :key="toast.id"
      :type="toast.kind === 'error' ? 'foreground' : 'background'"
      :duration="toast.duration ?? (toast.kind === 'error' ? Infinity : 5000)"
      class="overlay-toast"
      :class="`overlay-toast--${toast.kind}`"
      @update:open="(open) => open || remove(toast.id)"
    >
      <AppIcon :svg="icons[toast.kind]" size="var(--icon-size-sm)" class="overlay-toast__icon" />
      <div class="overlay-toast__text">
        <ToastTitle>{{ toast.message }}</ToastTitle>
        <ToastDescription v-if="toast.details !== undefined" as-child>
          <details class="overlay-toast__details">
            <summary>Details</summary>
            <ErrorDetails :error="toast.details" />
          </details>
        </ToastDescription>
      </div>
      <ToastClose as-child>
        <IconButton :icon="close" label="Dismiss" variant="ghost" size="sm" />
      </ToastClose>
    </ToastRoot>
    <ToastViewport class="overlay-toasts" />
  </ToastProvider>
</template>

<style>
/* Not scoped: toasts are rendered at the end of <body>. */
.overlay-confirm__message {
  margin: 0 0 var(--space-2);
}

.overlay-toasts {
  position: fixed;
  right: 0;
  bottom: 0;
  z-index: var(--z-toast);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  width: 380px;
  max-width: 100vw;
  margin: 0;
  padding: var(--space-4);
  list-style: none;
  outline: none;
}

.overlay-toast {
  display: flex;
  gap: var(--space-2);
  align-items: flex-start;
  padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
  border: var(--border-width) solid var(--color-border);
  border-left: 4px solid var(--toast-colour);
  border-radius: var(--radius-md);
  background: var(--color-surface-raised);
  box-shadow: var(--shadow-md);
  color: var(--color-text);
}

.overlay-toast--success {
  --toast-colour: var(--color-success);
}

.overlay-toast--info {
  --toast-colour: var(--color-info);
}

.overlay-toast--error {
  --toast-colour: var(--color-danger);
}

.overlay-toast__icon {
  margin-top: 5px;
  color: var(--toast-colour);
}

.overlay-toast__text {
  flex: 1;
  min-width: 0;
  padding-top: 4px;
}

.overlay-toast__details summary {
  color: var(--color-text-muted);
  font-size: var(--font-size-sm);
  cursor: pointer;
}

.overlay-toast__details[open] summary {
  margin-bottom: var(--space-1);
}
</style>
