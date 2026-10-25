<script setup lang="ts">
// A modal dialog, on Reka UI's Dialog (ADR-0013). While open it holds the
// focus, keeps Tab inside it and hides the rest of the page from assistive
// technology. It starts with the focus on its first field or button (the
// least destructive comes first by convention), and when it closes, the focus
// goes back to where it was.
//
// `title` names it. Escape, the close button and a click outside close it,
// unless `dismissible` is false, for work that mustn't be interrupted;
// `closeOnOutsideClick` turns off the click outside alone, and `beforeClose`
// can ask first, closing only if it answers true. With `alert` it is
// announced as an alert dialog, which needs a response. The default slot is
// the body and `footer` holds the buttons.
import closeIcon from '@material-symbols/svg-400/outlined/close.svg?raw'
import {
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from 'reka-ui'
import { computed, useTemplateRef } from 'vue'

import IconButton from './IconButton.vue'

const open = defineModel<boolean>('open', { default: false })
const props = withDefaults(
  defineProps<{
    title: string
    description?: string
    /** `full` fills the window. */
    size?: 'sm' | 'md' | 'lg' | 'full'
    dismissible?: boolean
    closeOnOutsideClick?: boolean
    alert?: boolean
    beforeClose?: () => Promise<boolean>
  }>(),
  {
    description: undefined,
    size: 'md',
    dismissible: true,
    closeOnOutsideClick: true,
    beforeClose: undefined,
  },
)

// Without a description, Reka UI would point aria-describedby at nothing.
const contentAttributes = computed(() => ({
  ...(props.description ? {} : { 'aria-describedby': undefined }),
  ...(props.alert ? { role: 'alertdialog' } : {}),
}))

async function close() {
  if (!props.beforeClose || (await props.beforeClose())) open.value = false
}

// Escape and a click outside close it here, so that `beforeClose` can ask.
function dismiss(event: Event, allowed = true) {
  event.preventDefault()
  if (props.dismissible && allowed) void close()
}

// Reka UI would start on the close button, the first thing in the dialog.
const body = useTemplateRef<HTMLElement>('body')
const footer = useTemplateRef<HTMLElement>('footer')
const FOCUSABLE =
  'input:not(:disabled), select:not(:disabled), textarea:not(:disabled), button:not(:disabled), [href], [tabindex]:not([tabindex="-1"])'

function onOpenAutoFocus(event: Event) {
  const first = [body.value, footer.value]
    .map((part) => part?.querySelector<HTMLElement>(FOCUSABLE))
    .find(Boolean)
  if (first) {
    event.preventDefault()
    first.focus()
  }
}
</script>

<template>
  <DialogRoot v-model:open="open">
    <DialogPortal>
      <DialogOverlay class="app-dialog__overlay" />
      <DialogContent
        class="app-dialog"
        :class="`app-dialog--${size}`"
        v-bind="contentAttributes"
        @escape-key-down="dismiss"
        @pointer-down-outside="dismiss($event, closeOnOutsideClick)"
        @open-auto-focus="onOpenAutoFocus"
      >
        <header class="app-dialog__header">
          <DialogTitle class="app-dialog__title">{{ title }}</DialogTitle>
          <IconButton
            v-if="dismissible"
            :icon="closeIcon"
            label="Close"
            variant="ghost"
            size="sm"
            @click="close"
          />
        </header>
        <DialogDescription v-if="description" class="app-dialog__description">
          {{ description }}
        </DialogDescription>
        <div ref="body" class="app-dialog__body"><slot /></div>
        <footer v-if="$slots.footer" ref="footer" class="app-dialog__footer">
          <slot name="footer" />
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<style>
/* Not scoped: the dialog is rendered at the end of <body>. */
.app-dialog__overlay {
  position: fixed;
  inset: 0;
  z-index: var(--z-dialog);
  background: var(--color-overlay);
}

.app-dialog {
  position: fixed;
  top: 50%;
  left: 50%;
  z-index: var(--z-dialog);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  width: min(var(--dialog-width), calc(100vw - 2 * var(--space-6)));
  max-height: calc(100vh - 2 * var(--space-6));
  padding: var(--space-4) var(--space-6) var(--space-6);
  border: var(--border-width) solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--color-surface-raised);
  box-shadow: var(--shadow-lg);
  color: var(--color-text);
  transform: translate(-50%, -50%);
}

.app-dialog--sm {
  --dialog-width: 400px;
}

.app-dialog--md {
  --dialog-width: 560px;
}

.app-dialog--lg {
  --dialog-width: 800px;
}

.app-dialog--full {
  inset: 0;
  width: auto;
  max-height: none;
  border: none;
  border-radius: 0;
  transform: none;
}

.app-dialog--full .app-dialog__body {
  flex: 1;
}

.app-dialog:focus-visible {
  outline: none;
}

.app-dialog__header {
  display: flex;
  gap: var(--space-2);
  align-items: center;
  justify-content: space-between;
}

.app-dialog__title {
  margin: 0;
  font-size: var(--font-size-xl);
}

.app-dialog__description {
  margin: 0;
  color: var(--color-text-muted);
}

/* It scrolls; the padding keeps its fields' focus rings from being cut off. */
.app-dialog__body {
  min-height: 0;
  margin: calc(-1 * var(--space-1));
  padding: var(--space-1);
  overflow: auto;
}

.app-dialog__footer {
  display: flex;
  gap: var(--space-2);
  justify-content: flex-end;
}
</style>
