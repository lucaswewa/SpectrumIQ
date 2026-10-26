# ADR-0048: The registration method

- **Status:** Accepted
- **Date:** 2026-10-25
- **Phase:** P5.1
- **Related:** ADR-0026, ADR-0047, REQ-STI-002, REQ-CAL-001, REQ-STI-004; `microscope`'s ADR-0023

## Context

- **Calibration and stitching ask one question:** how far has the content moved between two
  images?
  - The stage–camera calibration (P5.2) moves the stage by known amounts and measures the image's
    shift.
  - Stitching (M8) aligns overlapping tiles, at overlaps of 10–20 %.
- **What the answer must be:**
  - accurate to a tenth of a pixel or better, as calibration fits an affine to the shifts, and as
    REQ-STI-004 asks 0.3 px RMS of stitching;
  - right about which shift it is, when the overlap is small or the pattern repeats;
  - honest about when it can't tell;
  - quick at 100 MP frames' sizes.
- **`microscope`'s ADR-0023** chose phase correlation with a Hann window, a parabolic sub-pixel
  fit and a peak-ratio confidence. Its own radix-2 FFT was limited to power-of-two squares of
  ≤ 512, centred, ±128 shrunk pixels.

## Decision

- **Phase correlation, in `siq-register`** (L1), on `realfft` (rows) and `rustfft` (columns), at
  any size. Plans are kept per size in a `Registrar`.
- **The search:**
  - both images, less their means, tapered by a **Tukey window over a fifth of the image** (by
    default), which keeps the edges where low overlaps lie;
  - the cross-power spectrum, normalised to unit magnitude, **low-passed by a Gaussian** so that
    the peak is a Gaussian of one pixel and the noisiest frequencies weigh little;
  - the inverse transform peaks at the shift;
  - images with a side over `max_size` (512) are searched shrunk by block means.
- **Candidates:**
  - the 8 highest local maxima, each more than 2 pixels from a higher one;
  - each peak's shift is known only modulo the size, so it has **four hypotheses**: `dx` or
    `dx − width`, `dy` or `dy − height`;
  - the images' **normalised cross-correlation over each hypothesis's actual overlap** (at least
    2 % of the area) chooses within a peak;
  - across peaks, the **peak's height times that correlation** wins. Correlation alone chose a
    tiny overlap correlating by chance; height alone can't tell the four hypotheses apart.
- **The refinement:**
  - windows of the overlap at the whole-pixel shift, both showing the same content, at full
    resolution, up to 512 a side;
  - sides with no prime factor but 2, 3 and 5, so the transforms are quick and their plans few;
  - **tapered by Hann**, and correlated for the residual, the hypothesis nearest zero only;
  - repeated, up to three times, until the residual is under half a pixel.

  Each image tapered in its own coordinates pulls the peak toward zero: 0.27 px at 40 pixels'
  shift on 256 × 256. In windows that show the same content, the tapers agree.
- **Sub-pixel:** a **Gaussian through the peak and its two neighbours** on each axis, exact for
  the low-passed peak.
  - A parabola is also offered.
  - So is **Foroosh**'s estimate, which assumes the sharp, unwindowed peak and so wants the
    low-pass off.
- **Confidence** (`Registration`), each measured where it means something:
  - **`pce`,** the search's peak-to-correlation energy: the peak's square over the mean square of
    the surface outside it;
  - **`margin`,** the refinement's peak over its next one, from 0 to 1. In windows of the same
    content, a periodic pattern's copies stand as high as its true peak;
  - **`ncc`,** the overlap's correlation at the whole-pixel shift.
- **The shift's sign:** `b` shows at `(x, y)` what `a` shows at `(x − dx, y − dy)`. Moving the stage
  by `m` with the camera's axes aligned gives `−m` over the pixel size.

## Alternatives considered

- **Hann for the search, as `microscope` did:** a fifth of overlap at an edge is all but tapered
  away. The true peak fell out of the twelve highest at 20 % overlap.
- **A parabola on the sharp peak:** the whitened peak is close to a sinc. A parabola through
  three of its samples was off by up to 0.27 px, before the low-pass.
- **Foroosh as the default:** made for that sharp peak, but windows change its shape: 0.39 px at
  worst here.
- **A parabola through the correlation of the overlap at the 3 × 3 whole-pixel shifts** (no
  tapers at all): 0.12 px at worst on the known shifts, and 0.24 at low overlap. Blobs' broad
  correlation peaks are not parabolic.
- **Upsampled cross-correlation** (a matrix DFT around the peak): more precise in principle, more
  code, and the refined Gaussian is already within 0.04 px on camera frames.
- **Power-of-two squares, `microscope`'s:** a 20 % overlap of a 4096-pixel tile is 819 pixels by
  3072. Mixed radix takes it as it is.

## Consequences

- **Measured** (release, `register_pair`, simulated camera frames at 0.376 µm a pixel, with noise
  and vignetting):
  - 2048 × 1536 pairs moved 3 to 760 pixels were found within 0.009–0.034 px, in about 33 ms;
  - a 4096 × 3072 pair moved 1200 pixels, coarse to fine, within 0.010 px, in 82 ms;
  - PCE 11 000 to 4.8 million, margins 0.87 to 0.98.
- **On `siq-synth` patches with noise** (the tests):
  - 24 shifts within 0.1 px, RMS under 0.05;
  - low overlaps of a fifth within 0.2 px;
  - a periodic grating's margin 0.14;
  - unrelated patches' PCE 33, against 38 500 for a match.
- **Low overlap needs texture:** a 51-pixel strip of sparse cells held no peak to find. The PCE
  says so. Stitching crops its overlaps, and tiles' overlaps are hundreds of pixels.
- **NCC alone isn't a confidence** for blobby content: unrelated patches correlated at 0.70.
  Callers weigh `pce` and `margin` first, and set their thresholds (P5.2, M8).
- **Translation only.** Calibration fits rotation, scale and shear from several moves (P5.2).

## Verification

- `crates/siq-register/tests/register.rs`:
  - known shifts with noise;
  - large shifts coarse to fine;
  - low overlaps on grains;
  - periodic and unrelated images;
  - every option's accuracy;
  - inputs refused, `u16` frames, and plans reused.
- **`register_pair`:** the measurements above.
