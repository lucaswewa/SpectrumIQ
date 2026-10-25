import { computed, inject, type ComputedRef, type InjectionKey } from 'vue'

/** What a `FormField` tells the control inside it. */
export interface FieldContext {
  /** The control's id, which the field's label points at. */
  id: string
  /** The ids of the field's help and error texts, for `aria-describedby`. */
  describedBy: ComputedRef<string | undefined>
  /** Whether the field shows an error. */
  invalid: ComputedRef<boolean>
}

export const fieldKey: InjectionKey<FieldContext> = Symbol('field')

/**
 * The id, description and invalid state of a text, number or select control:
 * from its `FormField`, if it is in one. A control is invalid if it says so
 * itself (`invalid()`) or its field shows an error.
 */
export function useFieldControl(invalid: () => boolean) {
  const field = inject(fieldKey, undefined)
  return {
    id: field?.id,
    describedBy: computed(() => field?.describedBy.value),
    invalid: computed(() => invalid() || (field?.invalid.value ?? false)),
  }
}
