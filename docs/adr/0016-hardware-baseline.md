# ADR-0016: Hardware baseline and device roles

- **Status:** Accepted
- **Date:** 2026-10-23
- **Phase:** planning
- **Related:** REQ-ACQ-001, REQ-STG-001–005, REQ-DEV-001, REQ-NFR-002

## Context

These facts were established during planning:

- **Camera:** ~100 MP, 14-bit mono. It delivers Mono16 words and has ROI and binning modes with
  higher fps. The exact model is still to be confirmed.
- **Grabber:** Euresys Coaxlink; eGrabber 25.02 installed.
- **XYZ stage:** GRBL/grblHAL G-code over serial, typically open-loop steppers.
- **Focus drive:** separate, with a documented ASCII serial protocol.
- **Peripherals:** illumination, filter wheel, objective turret, and a light-field/confocal/SIM
  modality unit are all in scope. Their models and protocols are not chosen yet.
- **Workstation:** fixed and already working. Mini-ITX, AMD CPU, 32 GB RAM, RTX 3060 8 GB. The
  RTX 3060 has NVENC.

## Decision

- **Z roles:** the focus drive is the fine Z, used for z-stacks, autofocus and focus maps. The
  stage Z is the coarse Z, for sample height and clearance.
- **GRBL:** stop-and-go moves complete with `G4 P0` followed by an `Idle` status. Positions come
  from `?` polling, by default at 5 Hz. Positions are commanded, not measured, so backlash and
  repeatability are calibrated (P10.3).
- **The focus drive and future ASCII peripherals** use data-driven dialect files (P1.10).
- **Peripherals** are device traits with simulators now. Real drivers come one phase per device
  when the models are chosen (P10.5).
- **Every performance target** is set for this workstation.

## Alternatives considered

None: these are facts about the instrument.

## Consequences

- The open questions (camera model, focus drive protocol document, peripheral models, CPU core
  count, NVMe model) are tracked in [plan/README.md §6](../plan/README.md#6-open-questions).

## Verification

M10's bring-up runbooks and hardware tests.
