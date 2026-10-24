# ADR-0012: Simulation: protocol emulators, image-formation camera, ground-truth worlds

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-SIM-001–006

## Context

The goals require camera and stage simulation, so that development needs no hardware. They also
require simulated sample images and raw data for developing every algorithm. The drivers (GRBL, the
ASCII focus drive) should be tested even without hardware.

## Decision

- **`TrueState`** holds the true physical state: the stage pose with injected errors, focus, and
  the peripherals. The emulators update it, and the camera reads it.
- **Protocol-level emulators:** a GRBL 1.1 emulator, and a dialect-driven focus-drive emulator.
  The **real** drivers talk to them over in-memory duplex transports, or over a virtual COM port.
- **The camera simulator** renders from `siq-synth` worlds:
  - procedural 2D, 2.5D and 3D worlds, or a real image;
  - through an optics and sensor model: defocus, vignetting, noise, 14-bit, ROI and binning;
  - at realistic timing;
  - into the same frame pool and fan-out as the real camera.
- **`siq-datagen`** writes complete datasets with exact ground truth:
  - tile scans with known offsets;
  - z-stacks over known heights;
  - bead volumes;
  - SIM and HiLo sets;
  - light fields.
- **Euresys Playlink** (`playlink.cti`) is evaluated in P2.5, for testing the real eGrabber path
  without a camera.

## Alternatives considered

- **Trait-level mocks only:** don't exercise the drivers' protocol code.
- **Replaying recorded data only:** no ground truth, no stage dependence.

## Consequences

- The simulator code is significant (M1).
- A noiseless-render cache is needed to reach 9 fps at 100 MP when stationary.

## Verification

- P1.7–P1.10: the drivers pass against the emulators.
- M5, M8 and M11–M13: ground-truth accuracy tests.
