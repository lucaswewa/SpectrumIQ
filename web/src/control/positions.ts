// The saved positions are the server's, the `workflow` Thing's (ADR-0056): each by its name.

/** The first `Position n` not taken. */
export function nextName(taken: readonly { name: string }[]): string {
  for (let n = 1; ; n++) {
    const name = `Position ${n}`
    if (!taken.some((position) => position.name === name)) return name
  }
}
