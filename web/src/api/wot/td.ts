/**
 * The parts of a W3C Thing Description (TD 1.1) this app uses, as a
 * `teta-wot` server writes them.
 */

/** Where and how to perform an operation on an affordance. */
export interface Form {
  /** A URL, or a path resolved against the TD's `base`. */
  href: string
  /** The operations this form performs, such as `readproperty`. */
  op?: string | string[]
  contentType?: string
  /** `sse` for server-sent events. */
  subprotocol?: string
}

/** A JSON Schema-like description of a value. Unused keywords pass through. */
export interface DataSchema {
  type?: 'boolean' | 'integer' | 'number' | 'string' | 'object' | 'array' | 'null'
  title?: string
  description?: string
  unit?: string
  readOnly?: boolean
  writeOnly?: boolean
  default?: unknown
  enum?: unknown[]
  minimum?: number
  maximum?: number
  properties?: Record<string, DataSchema>
  items?: DataSchema
  required?: string[]
  [keyword: string]: unknown
}

export interface PropertyAffordance extends DataSchema {
  observable?: boolean
  forms: Form[]
}

export interface ActionAffordance {
  title?: string
  description?: string
  input?: DataSchema
  output?: DataSchema
  forms: Form[]
}

export interface EventAffordance {
  title?: string
  description?: string
  data?: DataSchema
  forms: Form[]
}

export interface ThingDescription {
  id?: string
  title: string
  description?: string
  /** The URL that relative `href`s are resolved against. */
  base?: string
  properties?: Record<string, PropertyAffordance>
  actions?: Record<string, ActionAffordance>
  events?: Record<string, EventAffordance>
  /** Forms for the whole Thing, such as `readallproperties`. */
  forms?: Form[]
  /** Other resources, such as MJPEG streams. */
  links?: Link[]
}

/** A link to another resource, such as a Thing's MJPEG stream. */
export interface Link {
  /** A URL, or a path resolved against the TD's `base`. */
  href: string
  /** Its media type, such as `multipart/x-mixed-replace`. */
  type?: string
  rel?: string
}

/**
 * The URL of the first HTTP form that performs `op`, resolved against
 * `base`, or undefined if none does. With `sse`, only server-sent event
 * forms count; without it, they don't. (WebSocket forms never count.)
 */
export function formUrl(
  forms: readonly Form[] | undefined,
  op: string,
  base: string | URL,
  { sse = false } = {},
): URL | undefined {
  for (const form of forms ?? []) {
    if (![form.op ?? []].flat().includes(op) || (form.subprotocol === 'sse') !== sse) continue
    const url = new URL(form.href, base)
    if (url.protocol === 'http:' || url.protocol === 'https:') return url
  }
  return undefined
}
