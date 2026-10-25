import about from '@material-symbols/svg-400/outlined/info.svg?raw'
import admin from '@material-symbols/svg-400/outlined/admin_panel_settings.svg?raw'
import calibration from '@material-symbols/svg-400/outlined/straighten.svg?raw'
import datasets from '@material-symbols/svg-400/outlined/dataset.svg?raw'
import jobs from '@material-symbols/svg-400/outlined/pending_actions.svg?raw'
import live from '@material-symbols/svg-400/outlined/videocam.svg?raw'
import logging from '@material-symbols/svg-400/outlined/receipt_long.svg?raw'
import mosaic from '@material-symbols/svg-400/outlined/auto_awesome_mosaic.svg?raw'
import scan from '@material-symbols/svg-400/outlined/grid_on.svg?raw'
import settings from '@material-symbols/svg-400/outlined/settings.svg?raw'
import threeD from '@material-symbols/svg-400/outlined/view_in_ar.svg?raw'
import viewer from '@material-symbols/svg-400/outlined/image.svg?raw'

/** A destination in the navigation rail, and its route. */
export interface Destination {
  /** The route's name, and its path (`/#/<id>`). */
  readonly id: string
  readonly label: string
  /** The icon's SVG source. */
  readonly icon: string
  /** Where it sits in the rail: with the workflows at the top, or pinned to the bottom. */
  readonly group: 'top' | 'bottom'
  /** The Things it needs: it's hidden while the instrument lacks any of them. */
  readonly requires: readonly string[]
  /** The milestone that builds its page, until it is built (docs/plan/README.md). */
  readonly planned?: string
}

/** The rail's destinations, in order (architecture §11): Shift+↑ and Shift+↓ follow it. */
export const DESTINATIONS: readonly Destination[] = [
  { id: 'live', label: 'Live', icon: live, group: 'top', requires: ['camera'], planned: 'M3' },
  {
    id: 'mosaic',
    label: 'Mosaic',
    icon: mosaic,
    group: 'top',
    requires: ['mosaic_live'],
    planned: 'M7',
  },
  { id: 'scan', label: 'Scan', icon: scan, group: 'top', requires: ['workflow'], planned: 'M6' },
  {
    id: 'datasets',
    label: 'Datasets',
    icon: datasets,
    group: 'top',
    requires: ['datasets'],
    planned: 'M4',
  },
  {
    id: 'viewer',
    label: 'Viewer',
    icon: viewer,
    group: 'top',
    requires: ['datasets'],
    planned: 'M4',
  },
  { id: '3d', label: '3D', icon: threeD, group: 'top', requires: ['datasets'], planned: 'M11' },
  { id: 'jobs', label: 'Jobs', icon: jobs, group: 'top', requires: ['jobs'], planned: 'M8' },
  {
    id: 'calibration',
    label: 'Calibration',
    icon: calibration,
    group: 'bottom',
    requires: ['calibration'],
    planned: 'M5',
  },
  { id: 'settings', label: 'Settings', icon: settings, group: 'bottom', requires: [] },
  { id: 'admin', label: 'Admin', icon: admin, group: 'bottom', requires: [], planned: 'M9' },
  { id: 'logging', label: 'Logging', icon: logging, group: 'bottom', requires: [] },
  { id: 'about', label: 'About', icon: about, group: 'bottom', requires: [] },
]
