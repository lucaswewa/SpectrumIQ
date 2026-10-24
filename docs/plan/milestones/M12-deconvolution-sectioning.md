# M12 — 3D: widefield deconvolution and optical sectioning

**Outcome:**

- PSF models;
- blockwise Richardson–Lucy deconvolution of large z-stacks, on CPU and GPU;
- pluggable optical-sectioning reconstructors (spinning-disk passthrough, 3-phase SIM, HiLo);
- volume rendering in VTK.js.

**Exit criteria:**

- on `siq-datagen` bead volumes, deconvolution improves the lateral FWHM by at least 30 % and the
  MSE against ground truth;
- the GPU and CPU results agree within 1e-3 relative;
- SIM and HiLo suppress out-of-focus light by at least 5× on synthetic ground truth;
- a 2 GB volume is deconvolved within budget, with RAM ≤ 14 GB and VRAM ≤ 4 GB.

---

### P12.1 — PSF models (`siq-deconv::psf`)

**Budget** ~700 · **Depends on** P0.2 · **Requirements** REQ-3D-002

**Goal.** Theoretical and measured PSFs.

**Scope**

- **Scalar Born–Wolf PSF** by numerical integration, with a J0 Bessel implementation.
- **Optionally, a Gibson–Lanni approximation** for refractive-index mismatch.
- **Parameters:** NA, wavelength, immersion and sample refractive indices, pixel size, z step,
  and size; the result is normalized.
- **Measured PSF from a bead stack:**
  - detect the beads;
  - refine each centre to sub-voxel precision;
  - crop, register and average;
  - subtract the background and normalize.

**Tests**

- Symmetry.
- Lateral FWHM ≈ 0.51 λ/NA.
- Axial extent.
- Measured PSF from simulated beads against the generating PSF.

**Example.** `psf_gen`.

**ADR.** PSF models and parameters.

---

### P12.2 — CPU Richardson–Lucy deconvolution (`siq-deconv::rl`)

**Budget** ~850 · **Depends on** P12.1, P4.2, P4.3 · **Requirements** REQ-3D-002, REQ-NFR-002

**Goal.** A correct, out-of-core reference implementation.

**Scope**

- **3D FFT convolution:** `realfft`/`rustfft` per axis, padded to fast sizes.
- **Richardson–Lucy,** with optional total-variation regularization and optional Biggs–Andrews
  acceleration.
- **Blockwise overlap-save over the volume Zarr:** the margins are the PSF half-size.
- **A job with an estimate,** and documented edge handling.

**Tests**

- Ground-truth beads: FWHM and MSE improvement.
- Flux conservation.
- No visible block seams (seam metric).

**Example.** `deconv_cpu`.

**ADR.** The deconvolution algorithm and blocking.

---

### P12.3 — GPU FFT and GPU Richardson–Lucy (`siq-gpu::fft`)

**Budget** ~950 · **Depends on** P12.2, P7.1 · **Requirements** REQ-3D-002, REQ-3D-005

**Goal.** Make deconvolution of 100 MP stacks practical on the RTX 3060.

**Scope**

- **WGSL Stockham FFT:**
  - radix-2 and radix-4, power-of-2 sizes;
  - batched 1D along each axis, giving 2D and 3D;
  - complex f32 in storage buffers.
- **Pointwise kernels** (multiply, divide, conjugate).
- **RL on GPU** per block, with the block size chosen from VRAM.
- Also usable by `siq-register` later.

**Tests**

- FFT against `rustfft` (relative error < 1e-4).
- GPU RL against CPU RL.
- Benchmarks.

**Example.** `fft_bench`.

**ADR.** The GPU FFT implementation.

---

### P12.4 — Optical sectioning reconstructors (`siq-section`)

**Budget** ~800 · **Depends on** P12.2, P4.9, P6.4 · **Requirements** REQ-3D-003, REQ-SIM-004

**Goal.** Support whichever sectioning hardware is chosen.

**Scope**

- **`SectioningReconstructor` trait:** inputs are the frames per plane (pattern phases) and the
  parameters; the output is a sectioned plane.
- **Implementations:**
  - `PassThrough`, for a spinning disk: rolling-ball or top-hat background subtraction, plus an
    optional deconvolution hook;
  - `ThreePhaseSim`: Neil et al.'s root-sum-of-squares, with a phase-error check;
  - `HiLo`: low frequencies from the local contrast of the structured image, high frequencies from
    the uniform image, merged at a σ cutoff.
- **`siq-datagen`** gains `sim-pattern` and `hilo` subcommands.
- **Workflow:** a step that acquires the pattern phases per z through `ModalityUnit` (simulated).

**Tests**

- Out-of-focus suppression ratio against ground truth.
- Behaviour with phase errors.
- Workflow integration.

**Example.** `section_demo`.

**ADR.** An amendment to ADR-0014 with the sectioning reconstructor design.

---

### P12.5 — Volume viewer (VTK.js)

**Budget** ~950 (web ~800, Rust endpoint ~150) · **Depends on** P11.4, P12.2 · **Requirements** REQ-VIS-003

**Goal.** Explore 3D volumes interactively, within browser limits.

**Scope**

- **Endpoint:** `/api/siq/data/{id}/volume?level&roi`, binary u16 with a JSON header, and a voxel
  cap per request.
- **Rendering:** `vtkVolume` with the volume mapper.
- **Transfer-function editor:** opacity and colour points, built from the `src/ui` wrappers.
- **Orthogonal slice views** (MPR).
- **Isosurfaces** computed client-side by `vtkImageMarchingCubes` on the downsampled level.
- **Level and ROI picker.**

**Tests**

- Decoding.
- Transfer-function store.
- Voxel-cap enforcement.
- Mount and unmount.

**Example.** `/#/dev/3d-volume`.

**ADR.** The volume visualization level-of-detail policy.
