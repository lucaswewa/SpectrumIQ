# ADR-0054: The autofocus algorithm and the focus surface model

- **Status:** Accepted
- **Date:** 2026-10-26
- **Phase:** P6.3
- **Related:** ADR-0026, ADR-0052, ADR-0053, REQ-WF-002; `microscope`'s ADR-0022 and ADR-0037

## Context

- **A scan is only as good as its focus.** A 10×/0.30 objective's depth of field is 7.4 µm
  (Berek's, in green, at 0.376 µm a pixel). A slide tilted by 0.03 rises that much over 250 µm,
  a few tiles. A scan must follow the slide, by autofocus at each tile or by a surface fitted
  to the focus found at a few places.
- **The focus drive is stepped:** the device trait moves to a position and says when it has
  arrived. It reports no speed, so a frame can't be placed on a moving drive by time.
- **The camera is large:** a full 100 MP frame is slow to take and to score, and near focus the
  little that the blur changes shows only at full resolution.
- **`microscope`'s autofocus** (its ADR-0022 and ADR-0037) sweeps its stage continuously while the
  preview streams, scores each frame by its JPEG size, and takes the centroid of the frames above
  half the curve's height. A curve that rises by less than 3 % has nothing to focus on, and the
  stage goes back where it began. Its centroid fixed the outliers that the sharpest frame gave:
  6 µm out, where the sweep missed a narrow peak.
- **The engine (ADR-0052)** runs plans as steps; the tile scan (ADR-0053) is made into steps when
  a run starts.

## Decision

- **Autofocus is two stepped sweeps** of the focus drive about where it is, each frame taken after
  the drive arrives, by `snap`:
  - **coarse:** 11 frames over the range (50 µm by default, 5 µm apart), binned 2 × 2 at a quarter
    of the exposure, so the light is the same;
  - **fine:** 13 frames over one and a half coarse steps either side of the coarse estimate
    (1.25 µm apart), at the camera's own settings;
  - **the camera** is put back at its own settings after, whatever happened.
- **Sharpness is Tenengrad** (`siq-imgproc`) over the frame's middle three quarters, away from the
  edges that vignetting darkens.
- **Each sweep's estimate is the vertex of a Gaussian:** a parabola fitted by least squares to the
  logarithms of the frames above half the sweep's peak, about the sharpest frame. The vertex is
  kept within those frames; if they don't make a peak, the sharpest frame is taken.
  - **The coarse estimate centres the fine sweep:** about the sharpest coarse frame instead, a
    flat top would leave the fine sweep lopsided, and its fit pulled to one side.
  - **The fine estimate is the focus,** and the drive goes there.
- **Nothing to focus on:** a coarse sweep whose sharpness rises by less than a fifth of its peak,
  `(max − min) / max < 0.2`, finds no focus, and the drive goes back where it was. The rise is the
  confidence returned with the focus and the curve.
- **In plans:**
  - `autofocus { range_um }` fails with nothing to focus on, and its error policy decides;
  - `focus_map { places, surface, range_um }` moves to each place and autofocuses there, a place
    with nothing to focus on left out, then fits the surface in force for the steps after it.
    Three places must remain;
  - `focus_on_surface { x, y }` moves the focus to the surface's z there;
  - a `tile_scan`'s `focus` is `none` (as it is), `fixed { z }`, `surface` (a `focus_on_surface`
    at each tile) or `autofocus { range_um }` (one at each tile);
  - each map, its places found and missed and its fitted surface, goes in the dataset's
    `layout.json` beside the grids;
  - a plan is refused before its dataset is made for an autofocus of no range, a map of fewer
    than three places, or a `focus_on_surface` before any map.
- **The focus surface:**
  - **`plane`:** least squares, then the places more than three robust deviations off it set
    aside (`3 × 1.4826 × MAD`, at least 0.5 µm), and fitted again, up to three times;
  - **`thin_plate`,** the default: a thin-plate spline through every place, in coordinates about
    their centre over their spread, which follows a tilt and a curve both;
  - **residuals:** each place's focus less the surface's. The spline goes through its places, so
    its residuals are each place's with it left out of the fit: how well the others predict it.

## Alternatives considered

- **`microscope`'s continuous sweep with a fan-out tap** (the brief's `Bounded` tap): faster, and
  it never stops the stream, but each frame's z would be interpolated from the drive's position
  by time, which the stepped trait doesn't give. Its own ADR-0022 found that timing errors
  biased it by up to a frame's worth of z. Stepped, each frame is where the drive said it is.
- **The centroid of the frames above half the peak** (`microscope`'s ADR-0037): it needs the curve
  symmetric within the frames above half. A sweep that starts or ends near focus cuts one side,
  and the centroid moves with it. The Gaussian's vertex doesn't need both sides.
- **The sharpest frame:** limited to the step, 2.5 µm here at 5 µm apart; and the coarse sweep's
  sharpest frame centred the fine sweep off focus on a flat top.
- **Golden-section search:** fewer frames when the curve is clean, but each probe depends on the
  last, noise sends it the wrong way, and it gives no curve to show or to judge confidence by.
- **One fine sweep over the whole range:** 41 frames at full resolution for 50 µm at 1.25 µm,
  against 24 frames, 11 of them binned.
- **Both sweeps binned:** quicker, but binning hides how little the blur changes within a few
  micrometres of focus, and the fine fit followed the noise.
- **Bilinear interpolation over a grid of places:** it needs a grid, and has a crease along each
  grid line. The spline takes any places and is smooth.
- **A plane only:** a curved slide, or a sample that sags, leaves it off by more than the depth of
  field at the edges, as the surface test shows; it stays an option for a slide known to be flat,
  with its outlier rejection.

## Consequences

- **Accurate on the simulator,** against the truth where the stage truly is:
  - on grains over a surface tilted by 0.03 and curved, from 3 to 20 µm out at five places,
    within a quarter of the depth of field (1.84 µm). At each place the error is the same from
    every start, within 0.08 µm, and between −1.03 and +0.31 µm across places. That is the
    tilt across the frame (3–7 µm over 96 µm) and where the grains are in it, not the search;
  - on a gentle relief at 512 × 384 (`autofocus_sim`), within 0.08 µm at five places;
  - a 5 × 5 scan with a 9-point thin-plate map: every tile within 0.97 µm.
- **Honest about empty fields:** a blank field rises by 7.9 %, from every start; a field with
  anything in it rises by 78 % or more. The threshold of 20 % has room either side, on the
  simulator; a real camera's noise may need another.
- **24 frames an autofocus,** about 5 s at 512 × 384 on the simulator, each after the drive
  arrives. A focus map of nine places costs nine of them; autofocus at every tile costs one a
  tile.
- **The focus found is the frame's, not its centre's:** where the sample's relief across the
  frame is more than the depth of field, as on the height field's default relief at 1024 × 768,
  autofocus finds the focus that sharpens the frame most, up to 6 µm from the centre's.
- **A cancel waits for an autofocus under way** to finish, up to its 24 frames, as it waits for a
  capture (ADR-0052): stopped halfway, it would leave the camera binned.
- **One sweep of the range:** a focus outside it is found at the range's edge, not followed.
  `microscope`'s looping autofocus would follow it, at more sweeps.
- **The simulator had to be right first:** its stage drew a new error for every axis at every
  move, so z moved by about 0.5 µm whenever x or y did, and a move out and back landed a rounding
  error off its target, which counted as moving. Each made the focus found at a place wrong at the
  tile. The emulator now lands on its targets, and draws an error only for the axes that moved.

## Verification

- `crates/siq-workflow/tests/autofocus.rs`, on the simulated instrument at 256 × 192 (the GRBL
  emulator, the simulator's instant focus drive):
  - `autofocus_finds_focus_within_a_quarter_of_the_depth_of_field`;
  - `a_bare_field_has_nothing_to_focus_on`, the drive back where it was and the step failing,
    and the plans refused;
  - `focus_surfaces_fit_their_places`: a plane exact and an outlier set aside, a spline within
    1 µm of a curved surface between its places, a plane not;
  - `a_5_by_5_scan_with_a_9_point_focus_map_is_in_focus_at_every_tile`.
- `crates/siq-sim/tests/grbl.rs`: an axis that doesn't move keeps its place, after a move out and
  back too.
- **`autofocus_sim`:** each sweep's curve, the focus against the truth, and a spline's residuals.
