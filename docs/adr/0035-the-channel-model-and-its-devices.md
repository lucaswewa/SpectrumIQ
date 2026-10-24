# ADR-0035: The channel model, and how it maps onto the devices

- **Status:** Accepted
- **Date:** 2026-10-24
- **Phase:** P2.4
- **Related:** ADR-0031, ADR-0032, ADR-0033, ADR-0034, REQ-DEV-001, REQ-SIM-003

## Context

- **A channel is what a user picks to image in:** brightfield, DAPI, GFP. The configuration's
  `[[channels]]` already describe each one (P0.3b): an id and a name, a light source and its
  intensity, a filter, an exposure, and a display colour.
- **Applying a channel moves several devices,** each with its own settle time. In the simulators
  (P1.10b), a light takes 5 ms, and the filter wheel 60 ms a position.
- **Frames carry an `AcquisitionContext`** (ADR-0031): the channel and the objective they were
  taken with, stamped at mid-exposure. A frame stamped with a channel must have been exposed in
  that channel only. It must not see the old light through the new filter, the new light through
  the old filter, or a filter turning.
- **Not every instrument has every peripheral,** and the real drivers come in P10.5.

## Decision

### What a channel holds

| Field | Device | Notes |
|---|---|---|
| `illumination: {source, intensity}` | `Illumination` | The source by its name in `[peripherals.illumination].sources`; the intensity from 0 to 1 |
| `filter` | `FilterWheel` | The filter by its name in `[peripherals.filter_wheel].filters`, not by position |
| `exposure` | The camera, through the acquisition engine | In ms |
| `colour` | The display mapping (ADR-0032) | `#ffffff` is grey; any other colour tints the previews |

- **Devices are named, not numbered,** so that rearranging a wheel changes only the
  `[peripherals]` section.
- **The objective and the modality are not part of a channel.** They have their own Things,
  `objective` and `modality`. The objective is stamped on frames in its own context field.

### Applying a channel

`imaging.select_channel(id)` is one hardware action, so it holds the global lock throughout
(ADR-0033). It takes these steps in order, each waiting for its device:

1. **The frames' channel is cleared.** Frames stamped from now on have none.
2. **Every lit source is turned off.**
3. **The filter is selected.** The wheel turns the shorter way round.
4. **The exposure is set,** from the next frame. The display's colormap is set to the channel's
   colour.
5. **The channel's source is set to its intensity,** and waits to settle.
6. **After half an exposure, at most 1 s, the frames' channel is set.** Frames are stamped at
   mid-exposure, so a frame stamped after that wait began its exposure after the light settled.

Then `imaging.channel` and the context's channel are both the new channel, and `channel_changed`
is emitted.

- **A missing device skips its step:** with no filter wheel, the filter is not changed.
- **An unknown source or filter name is an error.** The frames then have no channel, and
  `imaging.channel` stays `null` until a channel is applied.

### The peripherals' Things

| Thing | Properties | Actions | Events |
|---|---|---|---|
| `illumination` | `levels` (observable), `sources` | `set_level(source, level)`, by the source's name | `changed` |
| `filter_wheel` | `position`, `moving` (observable), `filters` | `select(position)` | `changed` |
| `objective` | `position`, `moving` (observable), `objectives`, `current` | `select(position)` | `changed` |
| `modality` | `modality`, `moving` (observable), `modes`, `patterns` | `set_modality(mode, pattern?)` | `changed` |

- **Each Thing is served only if its peripheral is fitted.** `imaging` is always served: a channel
  without peripherals still sets the exposure and the colour.
- **Each Thing mirrors its device's status watch** into observable properties, as the stage's
  Thing does (ADR-0033). `changed` follows each arrival.
- **`objective.select` labels frames as `select_channel` does:**
  - the frames' objective is cleared while the turret turns;
  - half an exposure after it arrives, the frames get the objective configured at that turret
    position;
  - a position with no configured objective leaves the frames without one.

## Alternatives considered

- **Filters by position in the channel:** shorter, but a channel would silently change meaning
  when a wheel is refitted.
- **Changing the filter and the light at once,** to save a light's settle time: a frame could see
  the new light through the old filter.
- **Labelling the frames as soon as the light settles:** a frame half exposed before then would
  carry the channel.
- **A channel that also sets the objective and modality:** a workflow (P6.1) sets those once per
  acquisition, and channels many times. Keeping them apart keeps a channel switch fast.
- **A `ChannelSwitcher` in `siq-acquire`:** the switch needs the peripherals, which the
  acquisition engine does not know about. It is service logic, in `siq-server`.

## Consequences

- **Every frame's channel can be trusted:** a frame has a channel only if it was exposed in that
  channel alone.
- **A channel switch costs:**
  - a light's settle time to turn the old light off;
  - the wheel's travel;
  - a light's settle time to turn the new light on;
  - half an exposure.

  From DAPI to GFP in the simulator, that is about 110 ms.
- **The workflows (P6.1) apply channels through the same `apply`,** under their own lock.
- **The real peripherals (P10.5) must report arrival truthfully,** because the order depends on
  their `select` and `set_level` waiting for it.

## Verification

`siq-server` `peripherals` tests:

- `a_channel_switch_darkens_the_old_light_before_the_filter_moves_and_lights_the_new_one_after`
  samples the true state throughout a switch from DAPI to GFP;
- `without_peripherals_only_imaging_is_added_and_a_channel_sets_the_camera`;
- `selecting_an_objective_labels_the_frames_with_it`;
- one test over HTTP for each peripheral's Thing.
