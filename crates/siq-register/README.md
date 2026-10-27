# siq-register

Image registration (ADR-0048): how far one image's content is from another's, to a fraction of a
pixel, and how sure that is, by phase correlation. The workhorse of the stage–camera calibration
(P5.2) and of stitching (M8).

- **The search:**
  - both images, less their means and tapered (Tukey by default, which keeps the edges where low
    overlaps lie), are transformed;
  - the cross-power spectrum, normalised and low-passed so that the peak is a Gaussian of a
    pixel, is transformed back;
  - the highest peaks are candidates, each with its four wrap hypotheses, which the images'
    normalised cross-correlation over each overlap chooses among;
  - images over 512 pixels a side are searched shrunk.
- **The refinement:** windows of the overlap that show the same content, at full resolution,
  tapered by Hann, until the residual is under half a pixel.
- **Sub-pixel:** a Gaussian through the peak and its neighbours (the default), a parabola, or
  Foroosh's estimate.
- **Confidence:** the search's peak-to-correlation energy, the refinement's margin of its peak over
  the next, and the overlap's correlation.

## Place in the architecture

- **Layer:** L1 Core.
- **Depends on:** `siq-types`; `realfft` and `rustfft` (the FFTs), `thiserror`.
- **Used by:** `siq-calib` and the calibration routine (P5.2), `siq-stitch` (M8).
- **Phase:** P5.1; P8.1 added the search's reach.

## API

| Item | What |
|---|---|
| `Registrar::new(Options)`, `default()` | A registrar, which keeps the FFT plans of each size it sees: `planned()` counts them |
| `registrar.register(a, b)` | How far `b`'s content is from `a`'s: two `ImageView`s of one size, of `u16` or `f32`, at least 16 pixels a side → `Registration` or `RegisterError` (`Sizes`, `TooSmall`, `Flat`, `Unreached`: no peak within the reach) |
| `Registration` | `shift` (`[dx, dy]`: `b` shows at `(x, y)` what `a` shows at `(x − dx, y − dy)`), `pce`, `margin`, `ncc` |
| `Options` | `window` (the search's: `Hann` or `Tukey(share)`), `subpixel` (`Gaussian`, `Parabolic`, `Foroosh`), `smoothing` (the peak's width in pixels; 0 for Foroosh), `candidates` (8), `max_size` (512), `min_overlap` (0.02 of the area), `reach` (the largest shift weighed each way, in pixels: any if `None`, the default) |

**Reading the confidence**, from the tests and the example:

- `pce` is in the thousands to millions for a match, about 30 for unrelated images, and about 150
  for a periodic grating.
- `margin` is near 1 for a match, and under 0.2 for a periodic pattern, whose shift is known only
  modulo its period.
- `ncc` is near 1 for a match. Blobby content can correlate by chance up to 0.7: weigh it with
  `pce`.

Callers set their own thresholds: P5.2 for calibration moves, M8 for tiles.

## Examples

| Example | What it shows | Run |
|---|---|---|
| `register_pair` | Pairs of simulated camera frames, the stage moved by known amounts up to a third of the frame, and a 4096 × 3072 pair coarse to fine: the shift found, its error, its confidence and the time | `cargo run --release -p siq-register --example register_pair` |

Every example also runs in smoke mode (`SIQ_SMOKE=1`) in `cargo xtask ci`.

## Features

None.

## Testing

`cargo test -p siq-register`, on `siq-synth` patches with noise:

- 24 known shifts, whole and fractional, up to 40 pixels: each within 0.1 px, RMS under 0.05;
- large shifts, up to 450 pixels on 1024 × 768, coarse to fine: within 0.1 px;
- low overlaps, a fifth of the image either way: within 0.2 px;
- a periodic grating (margin under 0.2) and unrelated images (PCE under 100), against a clear
  match (PCE over 1000);
- each option's accuracy: Gaussian or parabola with the low-pass within 0.1 px, Foroosh without it
  within 0.5;
- a shift of 60 pixels found within a reach of 70, and not within 30;
- the inputs refused, `u16` frames, and the plans reused.

## Limitations

- **Translation only:** rotation and scale aren't measured. The calibration fits them from several
  moves (P5.2).
- **The images must be the same size.** Stitching crops its overlaps first.
- **Low overlap needs texture:** a strip of sparse content may hold no peak to find. Check `pce`.
