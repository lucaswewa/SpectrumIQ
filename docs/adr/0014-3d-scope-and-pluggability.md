# ADR-0014: 3D reconstruction scope and pluggability

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-3D-001–005

## Context

The microscope supports, or will support:

- z-stacks for depth from focus;
- light-field imaging with a microlens array;
- optical sectioning hardware;
- widefield deconvolution.

The light-field geometry (conventional or Fourier) is **not decided**, and neither is the
sectioning hardware (spinning disk or structured illumination). Imaging is mixed and general
purpose: brightfield, reflected light and fluorescence, so channels are present throughout.

## Decision

- **DFF first** (M11). It is pure software, works with any specimen, and has the most direct
  deliverables: height map, all-in-focus image, confidence.
- **Deconvolution** (M12): PSF models (Born–Wolf, Gibson–Lanni) or a measured PSF; blockwise
  Richardson–Lucy on the CPU (reference) and the GPU (WGSL FFT).
- **Sectioning** (M12): a `SectioningReconstructor` trait with spinning-disk passthrough, 3-phase
  SIM and HiLo. The software-only methods come first. The hardware choice selects an
  implementation through configuration.
- **Light field** (M13): an `LfGeometry` trait covering MLA calibration, sub-aperture views,
  shift-and-sum refocus, and depth by DFF on the refocused stack. Conventional LFM comes first,
  then Fourier LFM. 3D light-field deconvolution is a stretch goal.
- **Execution:** all of it runs as jobs in the background pool (tokio orchestration, rayon CPU,
  a single GPU submission thread), admitted against the memory budget (ADR-0005).
- **The data model carries a channel axis everywhere** (`tczyx`).

## Alternatives considered

- **Committing to one light-field or sectioning geometry now:** premature, since the hardware is
  undecided.

## Consequences

The traits add a little abstraction cost. Each modality is validated on synthetic ground truth
before its hardware exists.

## Verification

The ground-truth targets in the exit criteria of M11, M12 and M13.
