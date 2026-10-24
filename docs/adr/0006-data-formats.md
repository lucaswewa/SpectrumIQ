# ADR-0006: Data formats: OME-Zarr master, raw scratch, OME-TIFF and DZI derived

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-DATA-001–006

## Context

- 14-bit data must be kept losslessly.
- Mosaics and volumes are large and multiscale, and carry channel, z and time axes.
- Interoperability with Fiji, napari and QuPath matters.
- OpenSeadragon consumes DZI, which is an 8-bit format.
- Only selected frames are saved; there is no continuous full-rate recording.

## Decision

- **Raw scratch:** `.raw` (u16 LE) plus a `.json` sidecar per frame, written with direct I/O from
  the pool slot. This is the acquisition format.
- **Master format:** OME-Zarr (NGFF) multiscale, `tczyx`, physical units in µm, zstd chunks. The
  version is fixed in P4.2: 0.5 unless a target reader fails, then 0.4.
- **Exports:** OME-TIFF (BigTIFF, tiled, pyramidal) for compatibility, and DZI-on-disk for static
  sharing.
- **Viewer tiles** are DZI rendered on demand from the OME-Zarr levels, with the requested LUT and
  cached.
- **Dataset folder** with `manifest.json` for provenance (architecture §8).

## Alternatives considered

- **OME-TIFF as master:** poor for parallel chunked writes and multiscale 3D.
- **HDF5:** a native C dependency, and a weaker browser and cloud story.
- **DZI as master:** 8-bit and lossy.

## Consequences

`zarrs` and zstd (native C through `zstd-sys`) are accepted dependencies (ADR-0007). An OME-TIFF
writer may have to be written by hand (P4.6).

## Verification

P4.2 and P4.6: round-trip tests, plus manual checks in napari, Fiji and QuPath, recorded.
