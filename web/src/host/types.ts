/** What the app knows about itself and where it runs. */
export interface AppInfo {
  /** The app's name. */
  readonly name: string
  /** The web app's version, from `package.json`. */
  readonly version: string
  /** Where the app runs: a browser tab, or (later) the Tauri desktop app. */
  readonly kind: 'browser' | 'tauri'
}

/** The state of a local SpectrumIQ service that the host manages. */
export type ServiceState = 'stopped' | 'starting' | 'running' | 'stopping' | 'failed'

/** Control of a local SpectrumIQ service, for hosts that manage one. */
export interface ServiceControl {
  /** The service's state now. */
  state(): Promise<ServiceState>
  /** Starts the service, if it isn't running. */
  start(): Promise<void>
  /** Stops the service gracefully. */
  stop(): Promise<void>
  /** Stops the service and starts it again. */
  restart(): Promise<void>
}

/**
 * The services the app needs from wherever it runs (ADR-0008).
 *
 * The app uses these instead of browser or Tauri APIs, so the same app runs
 * in a browser tab now and in the Tauri desktop app later. Only the host
 * implementation differs.
 */
/** Whether the app fills the screen, and switching it. */
export interface FullscreenControl {
  /** Whether it fills the screen now. */
  active(): boolean
  /** Fills the screen, or stops. */
  set(on: boolean): Promise<void>
  /** Calls `onChange` each time that changes, until the returned function is called. */
  watch(onChange: (active: boolean) => void): () => void
}

export interface HostAdapter {
  /** What the app knows about itself and where it runs. */
  readonly info: AppInfo
  /** Opens `url` outside the app: in a new browser tab, or the system browser. */
  openLink(url: string): void
  /** Saves `data` as a file called `name`: as a download, or through a save dialog. */
  saveFile(name: string, data: Blob): Promise<void>
  /** The app filling the screen: a browser's full screen, or the window's. */
  readonly fullscreen: FullscreenControl
  /**
   * Control of the local SpectrumIQ service, if this host manages one. A
   * browser tab doesn't; the Tauri desktop shell can (P3.5).
   */
  readonly service?: ServiceControl
}
