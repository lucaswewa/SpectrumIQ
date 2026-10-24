# ADR-0013: 2D acquisition and stitching strategy

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-WF-001, REQ-STI-001–004, REQ-CAL-001–002

## Context

- **The stage is GRBL and open-loop,** and reports position only through serial status polling.
  Timestamp matching during motion has several milliseconds of jitter, which means motion blur and
  position error at 100 MP.
- **The goals ask for two stitchers:**
  - Phase 1: real-time live stitching from calibration and stage pose;
  - Phase 2: post-acquisition stitching from stage heuristics plus phase correlation or SIFT/SURF,
    and a global bundle adjustment.

## Decision

- **Acquisition is stop-and-go first:** move, settle, capture. Continuous, position-triggered
  scanning gets a design ADR in P10.4, with later implementation if the hardware allows.
- **Live stitching** uses the deterministic affine (calibration × pose) and paints only stationary
  or slow frames. It is a navigation aid, not the final result.
- **Fine alignment is translation-only.** Rotation, scale and shear come from the per-objective
  calibration. Each pair is measured by stage-guided, coarse-to-fine phase correlation with
  sub-pixel peaks and NCC verification.
- **Global optimization:** weighted least squares over translations, with a weak stage prior,
  Huber IRLS and outlier rejection. This is the goals' pose graph, restricted to translations.
- **Feature matching** (pure Rust, ORB-like) is an optional fallback (P8.6). SURF is patented and
  excluded; SIFT is not needed.
- **Shading correction** uses calibration frames (dark and flat) when available, with
  retrospective BaSiC-style estimation as the fallback.
- **Blending:** linear feathering, out-of-core, into OME-Zarr.

## Alternatives considered

- **Per-tile affine:** less constrained, and needs strong features.
- **A lens-distortion model:** deferred; it can be added before alignment if residuals show radial
  structure.

## Consequences

Scans pay the settle time. In return, accuracy is good and simple to verify.

## Verification

P8.5: RMS ≤ 0.3 px and max ≤ 1 px on ground truth. P5.3: seam contrast reduced ≥ 5×.
