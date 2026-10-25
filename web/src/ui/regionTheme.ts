/**
 * The theme of the part of the page an element is in. A popup moved to
 * <body> takes it, so it matches its trigger: ADR-0011 lets any element set
 * the theme of its contents.
 */
export function regionTheme(element: Element | null | undefined): string | undefined {
  return element?.closest('[data-theme]')?.getAttribute('data-theme') ?? undefined
}
